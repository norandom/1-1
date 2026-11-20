//! Rewrites H.264 sequence parameter sets so decoders output every picture at once.
//!
//! RDP streams never reorder pictures: each AVC420 bitmap stream carries one picture
//! that must be shown now. An SPS without VUI `bitstream_restriction` leaves a decoder
//! to assume reordering, so it holds pictures back. This sets
//! `max_num_reorder_frames = 0` and `max_dec_frame_buffering = max_num_ref_frames`,
//! leaving every other field as sent.

/// GStreamer's H.264 base decoder (va, nvcodec) holds one picture back when the DPB
/// holds a single frame, as openh264's encoder declares; two frames avoid that.
/// (Measured with GStreamer 1.28.2, vah264dec on Intel and AMD.)
const MIN_BUFFERING: u64 = 2;

/// Returns `data` (Annex B) with every SPS rewritten, or `None` when nothing changed
/// or an SPS could not be parsed (the caller then passes the data on unchanged).
pub fn rewrite_annex_b(data: &[u8]) -> Option<Vec<u8>> {
    let nals = split_annex_b(data);
    if !nals
        .iter()
        .any(|&(start, end)| end > start && data[start] & 0x1f == 7)
    {
        return None;
    }
    let mut out = Vec::with_capacity(data.len() + 16);
    let mut changed = false;
    for (start, end) in nals {
        out.extend_from_slice(&[0, 0, 0, 1]);
        let nal = &data[start..end];
        if !nal.is_empty() && nal[0] & 0x1f == 7 {
            if let Some(rewritten) = rewrite_sps(nal) {
                out.extend_from_slice(&rewritten);
                changed = true;
                continue;
            }
        }
        out.extend_from_slice(nal);
    }
    changed.then_some(out)
}

/// (start, end) of each NAL payload after its start code.
fn split_annex_b(data: &[u8]) -> Vec<(usize, usize)> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(k, &start)| {
            let mut end = starts.get(k + 1).map_or(data.len(), |&next| next - 3);
            // A four-byte start code leaves a zero before the next one.
            while end > start && data[end - 1] == 0 {
                end -= 1;
            }
            (start, end)
        })
        .collect()
}

fn rewrite_sps(nal: &[u8]) -> Option<Vec<u8>> {
    let rbsp = unescape(&nal[1..]);
    let mut r = Reader {
        data: &rbsp,
        bit: 0,
    };
    let mut w = Writer::default();

    let profile_idc = r.copy(&mut w, 8)?;
    r.copy(&mut w, 16)?; // constraint flags, level_idc
    r.copy_ue(&mut w)?; // seq_parameter_set_id
    if matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        let chroma_format_idc = r.copy_ue(&mut w)?;
        if chroma_format_idc == 3 {
            r.copy(&mut w, 1)?;
        }
        r.copy_ue(&mut w)?;
        r.copy_ue(&mut w)?;
        r.copy(&mut w, 1)?;
        if r.copy(&mut w, 1)? == 1 {
            for i in 0..if chroma_format_idc == 3 { 12 } else { 8 } {
                if r.copy(&mut w, 1)? == 1 {
                    let size = if i < 6 { 16 } else { 64 };
                    let (mut last, mut next) = (8i64, 8i64);
                    for _ in 0..size {
                        if next != 0 {
                            let delta = r.copy_se(&mut w)?;
                            next = (last + delta + 256) % 256;
                        }
                        last = if next == 0 { last } else { next };
                    }
                }
            }
        }
    }
    r.copy_ue(&mut w)?; // log2_max_frame_num_minus4
    match r.copy_ue(&mut w)? {
        0 => {
            r.copy_ue(&mut w)?;
        }
        1 => {
            r.copy(&mut w, 1)?;
            r.copy_se(&mut w)?;
            r.copy_se(&mut w)?;
            for _ in 0..r.copy_ue(&mut w)? {
                r.copy_se(&mut w)?;
            }
        }
        _ => {}
    }
    let max_num_ref_frames = r.copy_ue(&mut w)?;
    r.copy(&mut w, 1)?; // gaps_in_frame_num_value_allowed_flag
    r.copy_ue(&mut w)?;
    r.copy_ue(&mut w)?;
    if r.copy(&mut w, 1)? == 0 {
        r.copy(&mut w, 1)?;
    }
    r.copy(&mut w, 1)?; // direct_8x8_inference_flag
    if r.copy(&mut w, 1)? == 1 {
        for _ in 0..4 {
            r.copy_ue(&mut w)?;
        }
    }

    // vui_parameters_present_flag
    if r.read(1)? == 0 {
        w.put(1, 1);
        // aspect, overscan, video signal, chroma loc, timing, nal hrd, vcl hrd, pic struct: absent.
        w.put(0, 8);
    } else {
        w.put(1, 1);
        if r.copy(&mut w, 1)? == 1 && r.copy(&mut w, 8)? == 255 {
            r.copy(&mut w, 32)?;
        }
        if r.copy(&mut w, 1)? == 1 {
            r.copy(&mut w, 1)?;
        }
        if r.copy(&mut w, 1)? == 1 {
            r.copy(&mut w, 4)?;
            if r.copy(&mut w, 1)? == 1 {
                r.copy(&mut w, 24)?;
            }
        }
        if r.copy(&mut w, 1)? == 1 {
            r.copy_ue(&mut w)?;
            r.copy_ue(&mut w)?;
        }
        if r.copy(&mut w, 1)? == 1 {
            r.copy(&mut w, 32)?;
            r.copy(&mut w, 32)?;
            r.copy(&mut w, 1)?;
        }
        let nal_hrd = r.copy(&mut w, 1)? == 1;
        if nal_hrd {
            copy_hrd(&mut r, &mut w)?;
        }
        let vcl_hrd = r.copy(&mut w, 1)? == 1;
        if vcl_hrd {
            copy_hrd(&mut r, &mut w)?;
        }
        if nal_hrd || vcl_hrd {
            r.copy(&mut w, 1)?;
        }
        r.copy(&mut w, 1)?; // pic_struct_present_flag
        if r.read(1)? == 1 {
            // An existing restriction: keep its motion-vector limits, replace the two counts.
            w.put(1, 1);
            r.copy(&mut w, 1)?;
            for _ in 0..4 {
                r.copy_ue(&mut w)?;
            }
            let reorder = r.read_ue()?;
            let buffering = r.read_ue()?;
            if std::env::var_os("PROBE_DEBUG").is_some() {
                eprintln!(
                    "SPS profile {profile_idc}: VUI restriction present, reorder {reorder}, buffering {buffering}, refs {max_num_ref_frames}"
                );
            }
            if reorder == 0 && buffering >= MIN_BUFFERING {
                return None; // already immediate output
            }
            w.put_ue(0);
            w.put_ue(buffering.max(max_num_ref_frames).max(MIN_BUFFERING));
            return Some(finish(nal[0], w));
        }
    }
    if std::env::var_os("PROBE_DEBUG").is_some() {
        eprintln!(
            "SPS profile {profile_idc}: no VUI restriction, refs {max_num_ref_frames}; adding one"
        );
    }
    // bitstream_restriction_flag and its fields.
    w.put(1, 1);
    w.put(1, 1); // motion_vectors_over_pic_boundaries_flag
    w.put_ue(2); // max_bytes_per_pic_denom (spec default)
    w.put_ue(1); // max_bits_per_mb_denom (spec default)
    w.put_ue(16); // log2_max_mv_length_horizontal (spec default)
    w.put_ue(16); // log2_max_mv_length_vertical (spec default)
    w.put_ue(0); // max_num_reorder_frames
    w.put_ue(max_num_ref_frames.max(MIN_BUFFERING)); // max_dec_frame_buffering
    Some(finish(nal[0], w))
}

fn copy_hrd(r: &mut Reader, w: &mut Writer) -> Option<()> {
    let count = r.copy_ue(w)? + 1;
    r.copy(w, 8)?;
    for _ in 0..count {
        r.copy_ue(w)?;
        r.copy_ue(w)?;
        r.copy(w, 1)?;
    }
    r.copy(w, 20)?;
    Some(())
}

fn finish(header: u8, mut w: Writer) -> Vec<u8> {
    w.put(1, 1); // rbsp_stop_one_bit
    while w.bits % 8 != 0 {
        w.put(0, 1);
    }
    let mut out = vec![header];
    out.extend(escape(&w.bytes));
    out
}

fn unescape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut zeros = 0;
    for &byte in data {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        out.push(byte);
    }
    out
}

fn escape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 4);
    let mut zeros = 0;
    for &byte in data {
        if zeros >= 2 && byte <= 3 {
            out.push(3);
            zeros = 0;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        out.push(byte);
    }
    out
}

struct Reader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl Reader<'_> {
    fn read(&mut self, n: u32) -> Option<u64> {
        let mut value = 0;
        for _ in 0..n {
            let byte = *self.data.get(self.bit / 8)?;
            value = value << 1 | u64::from(byte >> (7 - self.bit % 8) & 1);
            self.bit += 1;
        }
        Some(value)
    }

    fn read_ue(&mut self) -> Option<u64> {
        let mut zeros = 0;
        while self.read(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some((1 << zeros) - 1 + self.read(zeros)?)
    }

    fn copy(&mut self, w: &mut Writer, n: u32) -> Option<u64> {
        let value = self.read(n)?;
        w.put(value, n);
        Some(value)
    }

    fn copy_ue(&mut self, w: &mut Writer) -> Option<u64> {
        let value = self.read_ue()?;
        w.put_ue(value);
        Some(value)
    }

    fn copy_se(&mut self, w: &mut Writer) -> Option<i64> {
        let code = self.copy_ue(w)? as i64;
        Some(if code % 2 == 1 {
            (code + 1) / 2
        } else {
            -(code / 2)
        })
    }
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    bits: usize,
}

impl Writer {
    fn put(&mut self, value: u64, n: u32) {
        for i in (0..n).rev() {
            if self.bits % 8 == 0 {
                self.bytes.push(0);
            }
            let bit = (value >> i & 1) as u8;
            *self.bytes.last_mut().unwrap() |= bit << (7 - self.bits % 8);
            self.bits += 1;
        }
    }

    fn put_ue(&mut self, value: u64) {
        let code = value + 1;
        let length = 64 - code.leading_zeros();
        self.put(0, length - 1);
        self.put(code, length);
    }
}
