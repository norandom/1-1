//! Protection tests for `shared_frame.rs`, written from planted faults.

use super::*;

/// A rectangle with no area changes no pixel, so it leaves no damage for
/// the render thread to upload.
#[test]
fn an_empty_rectangle_leaves_no_damage() {
    let sf = SharedFrame::new(4, 4);

    // No width, one row tall.
    let mut guard = sf.begin_write();
    guard.update_rect(1, 1, 0, 1, &[0xAA; 8], 8);
    drop(guard);
    assert_eq!(sf.take_damage(), None, "no width, one row");

    // One pixel wide, no height.
    let mut guard = sf.begin_write();
    guard.update_rect(1, 1, 1, 0, &[0xAA; 8], 8);
    drop(guard);
    assert_eq!(sf.take_damage(), None, "one pixel wide, no rows");

    // A rectangle with area does leave damage, at its place.
    let mut guard = sf.begin_write();
    guard.update_rect(1, 1, 1, 1, &[0xAA; 4], 4);
    drop(guard);
    let damage = sf.take_damage().expect("a 1x1 rectangle is damage");
    assert_eq!((damage.x, damage.y, damage.width, damage.height), (1, 1, 1, 1));
}
