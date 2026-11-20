//! Prints NAL types of the first access units and whether the SPS rewrite fires.
use gstreamer as gst;
use gstreamer::prelude::*;
fn main() {
    gst::init().unwrap();
    for enc in ["openh264enc", "vah264lpenc b-frames=0"] {
        let p = gst::parse::launch(&format!("videotestsrc num-buffers=3 ! video/x-raw,width=320,height=240 ! videoconvert ! {enc} ! h264parse config-interval=-1 ! video/x-h264,stream-format=byte-stream,alignment=au ! appsink name=sink sync=false")).unwrap().downcast::<gst::Pipeline>().unwrap();
        let sink = p
            .by_name("sink")
            .unwrap()
            .downcast::<gstreamer_app::AppSink>()
            .unwrap();
        p.set_state(gst::State::Playing).unwrap();
        while let Ok(s) = sink.pull_sample() {
            let b = s.buffer().unwrap().map_readable().unwrap().to_vec();
            let types: Vec<u8> = b
                .windows(4)
                .enumerate()
                .filter(|(_, w)| w[..3] == [0, 0, 1])
                .map(|(i, _)| b[i + 3] & 0x1f)
                .collect();
            let rw = probe_gst_h264_decode::sps::rewrite_annex_b(&b);
            println!(
                "{enc}: {} bytes, NAL types {types:?}, rewritten: {}, again: {}",
                b.len(),
                rw.is_some(),
                rw.as_deref()
                    .and_then(probe_gst_h264_decode::sps::rewrite_annex_b)
                    .is_some()
            );
        }
        p.set_state(gst::State::Null).unwrap();
    }
}
