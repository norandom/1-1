use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

/// Immutable snapshot of a frame. The renderer holds an `Arc<FrameSnapshot>`
/// without blocking the session actor from writing new data.
pub struct FrameSnapshot {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// A changed region taken for upload: its pixels, tightly packed RGBA.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "read by the render thread, spec task 4.1")
)]
pub struct Damage {
    pub frame_width: u32,
    pub frame_height: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// The whole frame, at possibly new dimensions: the reader must
    /// (re)create its texture before uploading.
    pub full: bool,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Region {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl Region {
    fn union(self, other: Self) -> Self {
        Self {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }
}

struct SharedFrameInner {
    width: u32,
    height: u32,
    /// The authoritative picture. Never swapped out: readers copy from it.
    write_buf: Vec<u8>,
    /// Bounding box of the changes since the last `take_damage`.
    damage: Option<Region>,
    /// The frame was replaced (first frame, resize, reactivation).
    full: bool,
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
        let region = Region {
            x0: u32::from(x).min(self.inner.width),
            y0: u32::from(y).min(self.inner.height),
            x1: (u32::from(x) + u32::from(w)).min(self.inner.width),
            y1: (u32::from(y) + u32::from(h)).min(self.inner.height),
        };
        if region.x0 < region.x1 && region.y0 < region.y1 {
            self.inner.damage = Some(self.inner.damage.map_or(region, |d| d.union(region)));
        }
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

    /// Resize (when the dimensions changed) and overwrite the whole frame
    /// buffer in one locked step (task 9.2).
    ///
    /// A graphics-output reset resizes the session's decoded image upstream
    /// (`DecodedImage::reset_preserving_pointer`); the session loop calls
    /// this so the SharedFrame the GPU renderer reads is reallocated and
    /// fully repainted at the new size before any dirty-rect writes land.
    ///
    /// Returns `false` and leaves the frame untouched when `data` is shorter
    /// than `width * height * 4` (same silent-ignore contract as
    /// [`SharedFrame::update_full`]).
    pub fn update_full(&mut self, width: u32, height: u32, data: &[u8]) -> bool {
        let expected = (width as usize) * (height as usize) * 4;
        if data.len() < expected {
            return false;
        }
        if self.inner.width != width || self.inner.height != height {
            self.inner.width = width;
            self.inner.height = height;
            self.inner.write_buf.resize(expected, 0);
        }
        self.inner.write_buf[..expected].copy_from_slice(&data[..expected]);
        self.inner.full = true;
        self.inner.damage = Some(Region {
            x0: 0,
            y0: 0,
            x1: width,
            y1: height,
        });
        true
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
                damage: None,
                full: false,
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
        let mut guard = self.begin_write();
        let applied = guard.update_full(width, height, data);
        drop(guard);
        if applied {
            self.mark_dirty();
        }
    }

    /// Single rect update (for simple cases). For multiple rects, use begin_write().
    #[allow(dead_code)]
    pub fn update_rect(&self, x: u16, y: u16, w: u16, h: u16, rgba_data: &[u8], stride: usize) {
        let mut guard = self.begin_write();
        guard.update_rect(x, y, w, h, rgba_data, stride);
        drop(guard);
        self.mark_dirty();
    }

    /// A copy of the whole picture, if anything changed since the last
    /// publish. For tests and diagnostics; the renderer uses `take_damage`.
    pub fn publish(&self) -> Option<Arc<FrameSnapshot>> {
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return None;
        }
        let inner = self.inner.lock().unwrap();
        Some(Arc::new(FrameSnapshot {
            width: inner.width,
            height: inner.height,
            data: inner.write_buf.clone(),
        }))
    }

    /// The pixels of everything changed since the last take, as one
    /// rectangle; `None` when nothing changed. Holds the lock only to copy
    /// that rectangle.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "read by the render thread, spec task 4.1")
    )]
    pub fn take_damage(&self) -> Option<Damage> {
        let mut inner = self.inner.lock().unwrap();
        let region = inner.damage.take()?;
        let full = std::mem::take(&mut inner.full);
        Some(copy_region(&inner, region, full))
    }

    /// The whole picture, for a reader that has nothing yet (a session
    /// coming on screen). Leaves pending damage in place.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "read by the render thread, spec task 4.1")
    )]
    pub fn take_full(&self) -> Damage {
        let inner = self.inner.lock().unwrap();
        let region = Region {
            x0: 0,
            y0: 0,
            x1: inner.width,
            y1: inner.height,
        };
        copy_region(&inner, region, true)
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

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "read by the render thread, spec task 4.1")
)]
fn copy_region(inner: &SharedFrameInner, region: Region, full: bool) -> Damage {
    let (width, height) = (region.x1 - region.x0, region.y1 - region.y0);
    let row = width as usize * 4;
    let mut data = Vec::with_capacity(row * height as usize);
    for y in region.y0..region.y1 {
        let start = (y as usize * inner.width as usize + region.x0 as usize) * 4;
        data.extend_from_slice(&inner.write_buf[start..start + row]);
    }
    Damage {
        frame_width: inner.width,
        frame_height: inner.height,
        x: region.x0,
        y: region.y0,
        width,
        height,
        full,
        data,
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
    fn consecutive_publishes_keep_every_earlier_rect_b3_10_fixed() {
        // B3-10 (fixed 21 November 2026): publish() used to swap the write
        // buffer out and rebuild the next one from an older snapshot, so
        // partial updates published one by one lost the rects before them.
        // The buffer is now authoritative and never swapped.
        let sf = SharedFrame::new(2, 3); // 3 rows of 2 px

        paint_rect(&sf, 0, 0, 0xAA);
        let s1 = sf.publish().expect("s1");
        assert_eq!(&s1.data[..8], &[0xAA; 8]);

        paint_rect(&sf, 0, 1, 0xBB);
        let s2 = sf.publish().expect("s2");
        assert_eq!(&s2.data[..16], &[[0xAA; 8], [0xBB; 8]].concat()[..]);

        paint_rect(&sf, 0, 2, 0xCC);
        let s3 = sf.publish().expect("s3");
        assert_eq!(
            &s3.data[..],
            &[[0xAA; 8], [0xBB; 8], [0xCC; 8]].concat()[..]
        );
        // A snapshot still held by a reader does not change the next one.
        drop((s1, s2));
    }

    #[test]
    fn damage_is_the_union_of_changes_since_the_last_take() {
        let sf = SharedFrame::new(4, 4);
        assert_eq!(sf.take_damage(), None, "nothing changed yet");

        let mut guard = sf.begin_write();
        guard.update_rect(0, 0, 1, 1, &[1; 4], 4);
        guard.update_rect(2, 1, 1, 2, &[2; 8], 4);
        drop(guard);
        sf.mark_dirty();

        let damage = sf.take_damage().expect("damage");
        assert_eq!(
            (damage.x, damage.y, damage.width, damage.height, damage.full),
            (0, 0, 3, 3, false)
        );
        assert_eq!((damage.frame_width, damage.frame_height), (4, 4));
        // Rows of the 3x3 box, copied from the authoritative picture.
        assert_eq!(&damage.data[..12], &[[1; 4], [0; 4], [0; 4]].concat()[..]);
        assert_eq!(&damage.data[12..24], &[[0; 4], [0; 4], [2; 4]].concat()[..]);
        assert_eq!(damage.data.len(), 3 * 3 * 4);
        assert_eq!(sf.take_damage(), None, "taking clears the damage");
    }

    #[test]
    fn damage_is_clipped_to_the_frame() {
        let sf = SharedFrame::new(2, 2);
        sf.update_rect(1, 1, 4, 4, &[9; 64], 16);
        let damage = sf.take_damage().expect("damage");
        assert_eq!(
            (damage.x, damage.y, damage.width, damage.height),
            (1, 1, 1, 1)
        );
        sf.update_rect(5, 5, 1, 1, &[9; 4], 4);
        assert_eq!(sf.take_damage(), None, "outside the frame: nothing changed");
    }

    #[test]
    fn a_full_update_is_whole_frame_damage_at_the_new_size() {
        let sf = SharedFrame::new(2, 2);
        sf.update_rect(0, 0, 1, 1, &[1; 4], 4);
        sf.update_full(3, 1, &[7; 12]);
        let damage = sf.take_damage().expect("damage");
        assert!(damage.full);
        assert_eq!(
            (
                damage.frame_width,
                damage.frame_height,
                damage.width,
                damage.height
            ),
            (3, 1, 3, 1)
        );
        assert_eq!(damage.data, vec![7; 12]);

        sf.update_rect(1, 0, 1, 1, &[8; 4], 4);
        assert!(
            !sf.take_damage().unwrap().full,
            "later damage is partial again"
        );
    }

    #[test]
    fn take_full_gives_the_whole_picture_and_keeps_pending_damage() {
        let sf = SharedFrame::new(2, 1);
        sf.update_rect(1, 0, 1, 1, &[5; 4], 4);
        let full = sf.take_full();
        assert!(full.full);
        assert_eq!(full.data, [[0; 4], [5; 4]].concat());
        assert!(
            sf.take_damage().is_some(),
            "the pending damage is still there"
        );
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
