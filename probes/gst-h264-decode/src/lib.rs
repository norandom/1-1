//! A GStreamer-backed [`H264Decoder`] for the EGFX pipeline.
//!
//! One pipeline per decoder: `appsrc ! h264parse ! <decoder> ! appsink`.
//! Every call pushes one access unit and waits for its picture, because the
//! trait requires the picture of *this* call (AVC444 pairs the two views by
//! call order). A decoder that holds a picture back fails the call.

pub mod sps;

use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;
use ironrdp_egfx::decode::{DecodedFrame, DecoderError, DecoderResult, H264Decoder, Yuv420Frame};

/// Decoders in the order "auto" tries them.
pub const AUTO_ORDER: &[&str] = &[
    "vah264dec",
    "nvh264dec",
    "vaapih264dec",
    "openh264dec",
    "avdec_h264",
];

pub struct GstH264Decoder {
    element: String,
    pipeline: gst::Pipeline,
    src: gst_app::AppSrc,
    sink: gst_app::AppSink,
    frame: u64,
    timeout: Duration,
    ready: Option<bool>,
}

impl GstH264Decoder {
    /// Builds the pipeline around `element` (a decoder factory name such as
    /// `vah264dec` or `varenderD129h264dec`). Does not start it.
    pub fn new(element: &str, timeout: Duration) -> DecoderResult<Self> {
        gst::init().map_err(|e| DecoderError::new("GStreamer init", e))?;
        if gst::ElementFactory::find(element).is_none() {
            return Err(DecoderError::msg(format!(
                "GStreamer element {element} is not installed"
            )));
        }
        let parse = if std::env::var_os("PROBE_NO_PARSE").is_some() {
            ""
        } else {
            "! h264parse "
        };
        let options = low_latency_options(element);
        let description = format!(
            "appsrc name=src is-live=true format=time \
               caps=video/x-h264,stream-format=byte-stream,alignment=au \
             {parse}! {element} {options} \
             ! appsink name=sink sync=false max-buffers=8 \
               caps=video/x-raw,format={{NV12,I420}}"
        );
        let pipeline = gst::parse::launch(&description)
            .map_err(|e| DecoderError::new("pipeline", e))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| DecoderError::msg("not a pipeline"))?;
        let src = pipeline
            .by_name("src")
            .and_then(|e| e.downcast::<gst_app::AppSrc>().ok())
            .ok_or_else(|| DecoderError::msg("no appsrc"))?;
        let sink = pipeline
            .by_name("sink")
            .and_then(|e| e.downcast::<gst_app::AppSink>().ok())
            .ok_or_else(|| DecoderError::msg("no appsink"))?;
        Ok(Self {
            element: element.to_owned(),
            pipeline,
            src,
            sink,
            frame: 0,
            timeout,
            ready: None,
        })
    }

    /// The first installed decoder of [`AUTO_ORDER`] that starts.
    pub fn auto(timeout: Duration) -> Option<Self> {
        AUTO_ORDER.iter().find_map(|name| {
            let mut decoder = Self::new(name, timeout).ok()?;
            decoder.wait_until_ready().then_some(decoder)
        })
    }

    pub fn element(&self) -> &str {
        &self.element
    }

    fn bus_error(&self) -> Option<String> {
        let bus = self.pipeline.bus()?;
        bus.pop_filtered(&[gst::MessageType::Error])
            .map(|message| match message.view() {
                gst::MessageView::Error(error) => {
                    format!("{}: {}", error.error(), error.debug().unwrap_or_default())
                }
                _ => String::new(),
            })
    }

    fn push(&mut self, data: &[u8]) -> DecoderResult<()> {
        let mut data = to_annex_b(data);
        if std::env::var_os("PROBE_NO_SPS_REWRITE").is_none()
            && let Some(rewritten) = sps::rewrite_annex_b(&data)
        {
            data = rewritten;
        }
        let mut buffer = gst::Buffer::from_mut_slice(data);
        {
            let buffer = buffer.get_mut().expect("new buffer is writable");
            // Timestamps only need to increase; 60 fps spacing.
            let pts = gst::ClockTime::from_nseconds(self.frame * 16_666_667);
            buffer.set_pts(pts);
            buffer.set_dts(pts);
        }
        self.frame += 1;
        self.src.push_buffer(buffer).map(|_| ()).map_err(|e| {
            DecoderError::msg(format!(
                "push: {e:?} {}",
                self.bus_error().unwrap_or_default()
            ))
        })
    }

    fn pull(&mut self) -> DecoderResult<Yuv420Frame> {
        // The first picture also waits for the decoder to open its device.
        let timeout = if self.frame <= 1 {
            Duration::from_secs(3)
        } else {
            self.timeout
        };
        let Some(sample) = self
            .sink
            .try_pull_sample(gst::ClockTime::from_nseconds(timeout.as_nanos() as u64))
        else {
            return Err(DecoderError::msg(match self.bus_error() {
                Some(error) => format!("{}: {error}", self.element),
                None => format!(
                    "{} held the picture back (no output within {timeout:?})",
                    self.element
                ),
            }));
        };
        let caps = sample
            .caps()
            .ok_or_else(|| DecoderError::msg("sample without caps"))?;
        let info =
            gst_video::VideoInfo::from_caps(caps).map_err(|e| DecoderError::new("caps", e))?;
        let buffer = sample
            .buffer()
            .ok_or_else(|| DecoderError::msg("sample without buffer"))?;
        let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)
            .map_err(|_| DecoderError::msg("map frame"))?;
        to_yuv420(&frame)
    }

    /// Frames pushed but not yet pulled (for the bench).
    pub fn drain_late(&mut self) -> usize {
        let mut late = 0;
        while self
            .sink
            .try_pull_sample(gst::ClockTime::from_mseconds(50))
            .is_some()
        {
            late += 1;
        }
        late
    }
}

impl H264Decoder for GstH264Decoder {
    fn decode(&mut self, data: &[u8]) -> DecoderResult<DecodedFrame> {
        Ok(self.decode_yuv420(data)?.to_rgba())
    }

    fn decode_yuv420(&mut self, data: &[u8]) -> DecoderResult<Yuv420Frame> {
        self.push(data)?;
        self.pull()
    }

    fn supports_yuv420(&self) -> bool {
        true
    }

    fn wait_until_ready(&mut self) -> bool {
        if let Some(ready) = self.ready {
            return ready;
        }
        let ready =
            self.pipeline.set_state(gst::State::Playing).is_ok() && self.bus_error().is_none();
        self.ready = Some(ready);
        ready
    }

    fn reset(&mut self) {
        // A flush drops queued data; h264parse and the decoder restart on the next IDR.
        let _ = self.src.send_event(gst::event::FlushStart::new());
        let _ = self.src.send_event(gst::event::FlushStop::new(true));
    }
}

impl Drop for GstH264Decoder {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// Options that make a decoder output each picture as soon as it is decoded.
/// `PROBE_DEFAULT_OPTIONS=1` turns them off, to compare.
fn low_latency_options(element: &str) -> &'static str {
    if std::env::var_os("PROBE_DEFAULT_OPTIONS").is_some() {
        return "";
    }
    // GstH264Decoder subclasses (va, nvcodec, v4l2 stateless): bump the DPB early.
    if element.starts_with("va") && element.ends_with("h264dec") && element != "vaapih264dec"
        || element.starts_with("nv") && element.ends_with("h264dec")
    {
        return "compliance=flexible";
    }
    if element == "vaapih264dec" {
        return "low-latency=true";
    }
    ""
}

/// RDP servers send Annex B; some send 4-byte big-endian length prefixes.
fn to_annex_b(data: &[u8]) -> Vec<u8> {
    if data.starts_with(&[0, 0, 1]) || data.starts_with(&[0, 0, 0, 1]) {
        return data.to_vec();
    }
    let mut out = Vec::with_capacity(data.len() + 16);
    let mut rest = data;
    while rest.len() >= 4 {
        let len = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        let Some(nal) = rest.get(4..4 + len) else {
            return data.to_vec();
        };
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
        rest = &rest[4 + len..];
    }
    out
}

fn to_yuv420(frame: &gst_video::VideoFrameRef<&gst::BufferRef>) -> DecoderResult<Yuv420Frame> {
    let (width, height) = (frame.width() as usize, frame.height() as usize);
    let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
    let strides = frame.plane_stride();
    let plane = |i: u32| frame.plane_data(i).map_err(|_| DecoderError::msg("plane"));

    let mut y = Vec::with_capacity(width * height);
    let luma = plane(0)?;
    for row in 0..height {
        let start = row * strides[0] as usize;
        y.extend_from_slice(&luma[start..start + width]);
    }

    let mut u = Vec::with_capacity(chroma_width * chroma_height);
    let mut v = Vec::with_capacity(chroma_width * chroma_height);
    match frame.format() {
        gst_video::VideoFormat::Nv12 => {
            let uv = plane(1)?;
            for row in 0..chroma_height {
                let line = &uv[row * strides[1] as usize..][..chroma_width * 2];
                for pair in line.chunks_exact(2) {
                    u.push(pair[0]);
                    v.push(pair[1]);
                }
            }
        }
        gst_video::VideoFormat::I420 => {
            let (pu, pv) = (plane(1)?, plane(2)?);
            for row in 0..chroma_height {
                u.extend_from_slice(&pu[row * strides[1] as usize..][..chroma_width]);
                v.extend_from_slice(&pv[row * strides[2] as usize..][..chroma_width]);
            }
        }
        other => return Err(DecoderError::msg(format!("unexpected format {other:?}"))),
    }
    let to_u16 = |n: usize| u16::try_from(n).map_err(|_| DecoderError::msg("picture too large"));
    Yuv420Frame::new(
        to_u16(width)?,
        to_u16(height)?,
        y,
        width,
        u,
        v,
        chroma_width,
    )
}

/// The MS-RDPEGFX 3.3.8.3.1 reverse transform, as upstream's `Yuv420Frame::to_rgba`,
/// written so the compiler can vectorize it: preallocated output, two pixels per chroma
/// sample, no per-pixel bounds checks. Must equal upstream's output byte for byte.
pub fn fast_to_rgba(picture: &Yuv420Frame) -> Vec<u8> {
    let (width, height) = (usize::from(picture.width()), usize::from(picture.height()));
    let mut rgba = vec![0u8; width * height * 4];
    #[inline(always)]
    fn px(out: &mut [u8], luma: i32, ru: i32, gu: i32, bu: i32) {
        out[0] = (luma + ru).clamp(0, 255 << 8).wrapping_shr(8) as u8;
        out[1] = (luma + gu).clamp(0, 255 << 8).wrapping_shr(8) as u8;
        out[2] = (luma + bu).clamp(0, 255 << 8).wrapping_shr(8) as u8;
        out[3] = 0xFF;
    }
    for (row, out) in rgba.chunks_exact_mut(width * 4).enumerate() {
        let (Some(y), Some(u), Some(v)) = (
            picture.luma_row(row),
            picture.u_row(row / 2),
            picture.v_row(row / 2),
        ) else {
            break;
        };
        let (pairs, tail) = out.split_at_mut(width / 2 * 8);
        for (((out, y), &u), &v) in pairs
            .chunks_exact_mut(8)
            .zip(y.chunks_exact(2))
            .zip(u)
            .zip(v)
        {
            let (u, v) = (i32::from(u) - 128, i32::from(v) - 128);
            let (ru, gu, bu) = (403 * v, -48 * u - 120 * v, 475 * u);
            px(&mut out[..4], 256 * i32::from(y[0]), ru, gu, bu);
            px(&mut out[4..], 256 * i32::from(y[1]), ru, gu, bu);
        }
        if !tail.is_empty() {
            let (u, v) = (i32::from(u[width / 2]) - 128, i32::from(v[width / 2]) - 128);
            px(
                tail,
                256 * i32::from(y[width - 1]),
                403 * v,
                -48 * u - 120 * v,
                475 * u,
            );
        }
    }
    rgba
}
