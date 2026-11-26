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
    assert_eq!(
        (damage.x, damage.y, damage.width, damage.height),
        (1, 1, 1, 1)
    );
}

/// Data shorter than the new size is ignored: the frame keeps its size,
/// its pixels and its damage, and the call says it did nothing.
#[test]
fn data_too_short_for_a_new_size_leaves_the_frame_as_it_was() {
    let sf = SharedFrame::new(2, 2);

    let mut guard = sf.begin_write();
    let applied = guard.update_full(4, 4, &[0xEE; 4 * 4 * 4 - 1]);
    assert!(!applied, "too short is not applied");
    assert_eq!(
        (guard.width(), guard.height()),
        (2, 2),
        "the guard still sees 2x2"
    );
    drop(guard);
    assert_eq!(sf.dimensions(), (2, 2), "the size is kept");
    assert_eq!(sf.take_damage(), None, "nothing to upload");
    let whole = sf.take_full();
    assert_eq!((whole.frame_width, whole.frame_height), (2, 2));
    assert!(whole.data.iter().all(|&b| b == 0), "the pixels are kept");

    // The same through the frame itself: not marked dirty either.
    let before = sf.version();
    sf.update_full(4, 4, &[0xEE; 10]);
    assert_eq!(sf.dimensions(), (2, 2));
    assert_eq!(sf.version(), before);
}

/// A single rectangle written through the frame itself is announced: the
/// version goes up and the picture is published once.
#[test]
fn a_single_rect_update_marks_the_frame_dirty() {
    let sf = SharedFrame::new(2, 2);
    let before = sf.version();

    sf.update_rect(0, 0, 1, 1, &[0x11; 4], 4);

    assert_eq!(sf.version(), before + 1, "one change, one version");
    let snapshot = sf.publish().expect("published after update_rect");
    assert!(snapshot.data[..4].iter().all(|&b| b == 0x11));
    assert!(sf.publish().is_none(), "published once");
}
