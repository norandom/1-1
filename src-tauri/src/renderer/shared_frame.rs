use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Condvar, Mutex,
};

/// Immutable snapshot of a frame. The renderer holds an `Arc<FrameSnapshot>`
/// without blocking the session actor from writing new data.
pub struct FrameSnapshot {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

struct SharedFrameInner {
    width: u32,
    height: u32,
    write_buf: Vec<u8>,
    read_snapshot: Arc<FrameSnapshot>,
}

/// High-performance shared frame buffer between the RDP session actor and the GPU renderer.
///
/// Optimizations:
/// - Lock-free dirty check via AtomicBool (render thread doesn't lock when idle)
/// - Condvar wakes render thread on new frame (no busy-spin)
/// - `begin_write()`/`end_write()` batch multiple dirty rects under one lock
/// - `publish()` swaps buffers via `std::mem::swap` (zero-copy)
/// - `Arc::try_unwrap` reclaims old snapshot's allocation
pub struct SharedFrame {
    inner: Mutex<SharedFrameInner>,
    dirty: AtomicBool,
    version: AtomicU64,
    /// Condvar to wake the render thread when a new frame is published
    notify: Condvar,
}

/// Guard for batched writes. Holds the mutex for the duration of multiple update_rect calls.
/// Drops automatically, releasing the lock.
pub struct WriteGuard<'a> {
    inner: std::sync::MutexGuard<'a, SharedFrameInner>,
}

#[allow(dead_code)]
impl WriteGuard<'_> {
    /// Write a dirty rectangle into the frame buffer.
    /// `rgba_data` starts at the rect's first pixel; rows are `stride` bytes apart.
    pub fn update_rect(&mut self, x: u16, y: u16, w: u16, h: u16, rgba_data: &[u8], stride: usize) {
        let fw = self.inner.width as usize;
        let fh = self.inner.height as usize;
        let row_bytes = w as usize * 4;

        for row in 0..h as usize {
            let dy = y as usize + row;
            if dy >= fh {
                break;
            }
            let src_offset = row * stride;
            let dst_offset = (dy * fw + x as usize) * 4;
            if src_offset + row_bytes <= rgba_data.len()
                && dst_offset + row_bytes <= self.inner.write_buf.len()
            {
                self.inner.write_buf[dst_offset..dst_offset + row_bytes]
                    .copy_from_slice(&rgba_data[src_offset..src_offset + row_bytes]);
            }
        }
    }

    pub fn width(&self) -> u32 {
        self.inner.width
    }
    pub fn height(&self) -> u32 {
        self.inner.height
    }
}

impl SharedFrame {
    pub fn new(width: u32, height: u32) -> Arc<Self> {
        let size = (width * height * 4) as usize;
        Arc::new(Self {
            inner: Mutex::new(SharedFrameInner {
                width,
                height,
                write_buf: vec![0u8; size],
                read_snapshot: Arc::new(FrameSnapshot {
                    width,
                    height,
                    data: vec![0u8; size],
                }),
            }),
            dirty: AtomicBool::new(false),
            version: AtomicU64::new(0),
            notify: Condvar::new(),
        })
    }

    /// Begin a batched write. Locks the mutex ONCE for multiple update_rect calls.
    /// Call `end_write()` when done to mark dirty and wake the renderer.
    pub fn begin_write(&self) -> WriteGuard<'_> {
        WriteGuard {
            inner: self.inner.lock().unwrap(),
        }
    }

    /// Mark the frame as dirty and wake the render thread.
    /// Call after dropping the WriteGuard or after update_full.
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
        self.version.fetch_add(1, Ordering::Relaxed);
        self.notify.notify_one();
    }

    /// Called by the session actor to update the entire frame.
    #[allow(dead_code)]
    pub fn update_full(&self, width: u32, height: u32, data: &[u8]) {
        let mut inner = self.inner.lock().unwrap();
        let expected = (width * height * 4) as usize;
        if data.len() < expected {
            return;
        }
        if inner.width != width || inner.height != height {
            inner.width = width;
            inner.height = height;
            inner.write_buf.resize(expected, 0);
        }
        inner.write_buf[..expected].copy_from_slice(&data[..expected]);
        drop(inner);
        self.mark_dirty();
    }

    /// Single rect update (for simple cases). For multiple rects, use begin_write().
    #[allow(dead_code)]
    pub fn update_rect(&self, x: u16, y: u16, w: u16, h: u16, rgba_data: &[u8], stride: usize) {
        let mut guard = self.begin_write();
        guard.update_rect(x, y, w, h, rgba_data, stride);
        drop(guard);
        self.mark_dirty();
    }

    /// Publish the write buffer as a new immutable snapshot.
    /// Zero-copy: swaps write_buf into snapshot, reclaims old snapshot's Vec.
    pub fn publish(&self) -> Option<Arc<FrameSnapshot>> {
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return None;
        }
        let mut inner = self.inner.lock().unwrap();

        let mut snapshot_data = Vec::new();
        std::mem::swap(&mut inner.write_buf, &mut snapshot_data);

        let snapshot = Arc::new(FrameSnapshot {
            width: inner.width,
            height: inner.height,
            data: snapshot_data,
        });

        let old_snapshot = std::mem::replace(&mut inner.read_snapshot, snapshot.clone());

        // Reclaim old Vec allocation if renderer released it
        match Arc::try_unwrap(old_snapshot) {
            Ok(old) => {
                inner.write_buf = old.data;
                let expected = (inner.width * inner.height * 4) as usize;
                inner.write_buf.resize(expected, 0);
            }
            Err(_) => {
                let expected = (inner.width * inner.height * 4) as usize;
                inner.write_buf = vec![0u8; expected];
            }
        }

        Some(snapshot)
    }

    /// Block until a new frame is available or timeout (for render thread).
    pub fn wait_for_frame(&self, timeout: std::time::Duration) -> bool {
        if self.dirty.load(Ordering::Acquire) {
            return true;
        }
        let guard = self.inner.lock().unwrap();
        let (_guard, result) = self.notify.wait_timeout(guard, timeout).unwrap();
        !result.timed_out() || self.dirty.load(Ordering::Acquire)
    }

    #[allow(dead_code)]
    pub fn dimensions(&self) -> (u32, u32) {
        let inner = self.inner.lock().unwrap();
        (inner.width, inner.height)
    }

    #[allow(dead_code)]
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }
}

/// Characterization tests for the GPU shared frame buffer (task 3.4,
/// Req 4.2 — the "GPU render surface synchronized behind the webview"
/// seam). All tests are synchronous and deterministic: no GPU, no
/// windows, no load-bearing sleeps (the wake test joins a condvar-notified
/// thread; it carries one 20 ms park-hint sleep whose outcome is
/// timing-independent via the dirty-flag fast path — a wake that fires
/// before the park still returns through `dirty.load()`).
#[cfg(test)]
mod tests {
    use super::*;

    /// Paint a 2x1 rect (8 bytes) of a single value at (x, y).
    fn paint_rect(sf: &SharedFrame, x: u16, y: u16, value: u8) {
        let mut guard = sf.begin_write();
        guard.update_rect(x, y, 2, 1, &[value; 8], 8);
        drop(guard);
        sf.mark_dirty();
    }

    #[test]
    fn update_rect_writes_rows_at_the_rect_position() {
        let sf = SharedFrame::new(4, 2);

        let mut guard = sf.begin_write();
        guard.update_rect(1, 0, 2, 1, &[0xAA; 8], 8); // rect (1,0) 2x1
        guard.update_rect(1, 1, 2, 1, &[0xBB; 8], 8); // rect (1,1) 2x1
        drop(guard);
        sf.mark_dirty();

        let snapshot = sf.publish().expect("dirty after update_rect");
        assert_eq!(snapshot.width, 4);
        assert_eq!(snapshot.height, 2);
        // Row 0 pixels (1,2) = AA; row 1 pixels (1,2) = BB; rest zero.
        for y in 0..2u32 {
            for x in 0..4u32 {
                let expected = if y == 0 && (x == 1 || x == 2) {
                    0xAA
                } else if y == 1 && (x == 1 || x == 2) {
                    0xBB
                } else {
                    0
                };
                let px = &snapshot.data[((y * 4 + x) * 4) as usize..][..4];
                assert!(px.iter().all(|&b| b == expected), "pixel ({x},{y})");
            }
        }
    }

    #[test]
    fn update_rect_clamps_rows_beyond_the_frame_height() {
        let sf = SharedFrame::new(2, 1);

        // 3 rows tall starting at y=1: rows 1 and 2 are out of bounds.
        let mut guard = sf.begin_write();
        guard.update_rect(0, 1, 2, 3, &[0xCC; 24], 8);
        drop(guard);
        sf.mark_dirty();

        let snapshot = sf.publish().expect("dirty");
        assert!(
            snapshot.data.iter().all(|&b| b == 0),
            "no row landed in bounds"
        );
    }

    #[test]
    fn update_rect_skips_rows_that_overflow_the_frame_width() {
        let sf = SharedFrame::new(2, 1);

        // x=2 with a 2px-wide row: dst offset (0*2+2)*4 = 8 == buffer len,
        // so the whole row falls outside the buffer and is skipped.
        let mut guard = sf.begin_write();
        guard.update_rect(2, 0, 2, 1, &[0xDD; 8], 8);
        drop(guard);
        sf.mark_dirty();

        let snapshot = sf.publish().expect("dirty");
        assert!(
            snapshot.data.iter().all(|&b| b == 0),
            "row skipped silently"
        );
    }

    #[test]
    fn publish_without_dirty_returns_none_and_version_tracks_marks() {
        let sf = SharedFrame::new(2, 1);

        assert!(sf.publish().is_none(), "fresh frame is clean");
        assert_eq!(sf.version(), 0);

        sf.mark_dirty();
        sf.mark_dirty();
        assert_eq!(sf.version(), 2);
        assert!(sf.publish().is_some());
        assert!(sf.publish().is_none(), "publish clears the dirty flag");
        assert_eq!(sf.version(), 2, "publish does not bump the version");
    }

    #[test]
    fn rects_accumulate_within_one_publish_batch() {
        let sf = SharedFrame::new(2, 2);

        // Both rects land in the same write batch before one publish.
        let mut guard = sf.begin_write();
        guard.update_rect(0, 0, 2, 1, &[0x11; 8], 8);
        guard.update_rect(0, 1, 2, 1, &[0x22; 8], 8);
        drop(guard);
        sf.mark_dirty();

        let snapshot = sf.publish().expect("dirty after the batch");
        assert_eq!(&snapshot.data[..8], &[0x11; 8]);
        assert_eq!(&snapshot.data[8..], &[0x22; 8]);
    }

    #[test]
    fn consecutive_publishes_lose_the_other_branchs_rects_defect_b3_10() {
        // B3-10: publish() swaps the write buffer out and rebuilds the next
        // one from the PREVIOUS published snapshot (or zeros), so partial
        // updates published one-by-one alternate between two accumulation
        // branches — every snapshot loses the rects of the publish before
        // it. The GPU surface therefore diverges from the remote display
        // for any multi-PDU sequence (the actor writes only the dirty
        // rects of each PDU). Pinned as a defect (Req 4.2 sync); a fix must
        // keep the full accumulated surface in every snapshot.
        let sf = SharedFrame::new(2, 3); // 3 rows of 2 px

        paint_rect(&sf, 0, 0, 0xAA); // row 0
        let s1 = sf.publish().expect("s1");
        assert_eq!(&s1.data[..8], &[0xAA; 8]);
        drop(s1);

        paint_rect(&sf, 0, 1, 0xBB); // row 1
        let s2 = sf.publish().expect("s2");
        assert_eq!(&s2.data[8..16], &[0xBB; 8], "the new rect is present");
        assert_eq!(&s2.data[..8], &[0x00; 8], "row 0 from publish #1 is LOST");
        drop(s2);

        paint_rect(&sf, 0, 2, 0xCC); // row 2
        let s3 = sf.publish().expect("s3");
        assert_eq!(&s3.data[16..], &[0xCC; 8], "the new rect is present");
        assert_eq!(&s3.data[..8], &[0xAA; 8], "row 0 returns via the reclaim");
        assert_eq!(&s3.data[8..16], &[0x00; 8], "row 1 from publish #2 is LOST");
    }

    #[test]
    fn publish_while_previous_snapshot_is_referenced_zeroes_the_next_base_defect_b3_10() {
        // B3-10 (referenced-snapshot half): when the renderer still holds
        // the previous snapshot Arc, publish() cannot reclaim its buffer
        // and starts the next write buffer ZEROED — even the branch base is
        // lost. Pinned as a defect; a fix must keep the accumulated surface.
        let sf = SharedFrame::new(2, 3);

        paint_rect(&sf, 0, 0, 0xAA);
        let held = sf.publish().expect("s1, still referenced below");

        paint_rect(&sf, 0, 1, 0xBB);
        let s2 = sf.publish().expect("s2");
        assert_eq!(&s2.data[8..16], &[0xBB; 8]);
        drop(s2);

        paint_rect(&sf, 0, 2, 0xCC);
        let s3 = sf.publish().expect("s3");
        assert_eq!(&s3.data[16..], &[0xCC; 8], "the new rect is present");
        assert_eq!(
            &s3.data[..16],
            &[0u8; 16],
            "rows 0 and 1 both lost: the base was zeroed under contention"
        );
        drop(held);
    }

    #[test]
    fn update_full_replaces_content_and_resizes() {
        let sf = SharedFrame::new(2, 1);
        sf.update_full(1, 1, &[0x77; 4]);

        let snapshot = sf.publish().expect("dirty after update_full");
        assert_eq!(snapshot.width, 1);
        assert_eq!(snapshot.height, 1);
        assert_eq!(snapshot.data, vec![0x77; 4]);
    }

    #[test]
    fn update_full_with_short_data_is_silently_ignored() {
        let sf = SharedFrame::new(2, 1);
        let before = sf.version();

        sf.update_full(2, 1, &[0x01, 0x02]); // needs 8 bytes

        assert_eq!(sf.version(), before, "not even marked dirty");
        assert!(sf.publish().is_none(), "no visible effect");
    }

    #[test]
    fn wait_for_frame_returns_immediately_when_dirty() {
        let sf = SharedFrame::new(2, 1);
        sf.mark_dirty();

        let start = std::time::Instant::now();
        assert!(sf.wait_for_frame(std::time::Duration::from_secs(60)));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "fast path must not wait out the timeout"
        );
    }

    #[test]
    fn wait_for_frame_parks_until_mark_dirty_wakes_it() {
        let sf = SharedFrame::new(2, 1);
        let waiter_sf = sf.clone();
        let waiter = std::thread::spawn(move || {
            waiter_sf.wait_for_frame(std::time::Duration::from_secs(60))
        });

        // Give the waiter a moment to park; it cannot return early because
        // the frame is clean (it blocks on the condvar until mark_dirty).
        std::thread::sleep(std::time::Duration::from_millis(20));
        sf.mark_dirty();

        let woke = waiter
            .join()
            .expect("waiter thread must not panic within 60 s");
        assert!(woke, "condvar wake beats the timeout");
    }
}
