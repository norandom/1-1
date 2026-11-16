//! Task 9.4 (design L316; Reqs 4.1/4.4/9.8): pure dirty-region batching
//! math for the session drain.
//!
//! After `ActiveStage::process` drains a PDU batch, every
//! [`ActiveStageOutput::GraphicsUpdate`] region of that ONE batch is merged
//! into a single bounding rectangle; the packet then carries that rect's
//! pixels extracted from the authoritative composited framebuffer (the
//! decoded image upstream already composited during `process`), so
//! unchanged pixels inside the bounds come along from the framebuffer and
//! accumulated dirty regions are never lost (Req 9.8). Kind selection
//! follows the design contract: kind 1 (full frame) for the initial
//! delivery on a surface, a resize, or full-surface invalidation; kind 2
//! (dirty rect) otherwise. Existing packet layouts remain unchanged.
//!
//! std rectangle math only — no new buffering crate (task boundary).

use ironrdp_pdu::geometry::InclusiveRectangle;

/// Delivery plan for one drained update batch (design L316).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchPlan {
    /// Emit a kind-1 full frame: initial delivery on this surface
    /// (`delivered` is `None` until the first frame after (re)connect), a
    /// resize (delivered surface dims differ from the decoded image), or
    /// full-surface invalidation (the merged damage covers every pixel —
    /// upstream `widen_to_full_refresh` after a graphics reset). Otherwise
    /// emit one kind-2 dirty rect.
    pub full_refresh: bool,
    /// The merged bounding rectangle of ALL drained dirty regions,
    /// clamped to the decoded surface.
    pub rect: InclusiveRectangle,
}

/// Plan one drained update batch.
///
/// `surface` is the decoded image size; `delivered` is the surface size
/// already delivered to the UI on this connection (`None` before the
/// first delivery). Returns `None` when no region intersects the surface
/// (nothing to composite) — upstream clips damage to the image, so this
/// is purely defensive.
pub fn plan_batch(
    regions: &[InclusiveRectangle],
    surface: (u16, u16),
    delivered: Option<(u16, u16)>,
) -> Option<BatchPlan> {
    let rect = union_clamped(regions, surface)?;
    let full_refresh = delivered != Some(surface) || covers_full_surface(rect.clone(), surface);
    Some(BatchPlan { full_refresh, rect })
}

/// Full-surface invalidation: the merged damage covers every pixel of the
/// decoded surface.
pub fn covers_full_surface(rect: InclusiveRectangle, surface: (u16, u16)) -> bool {
    rect.left == 0
        && rect.top == 0
        && rect.right == surface.0.saturating_sub(1)
        && rect.bottom == surface.1.saturating_sub(1)
}

/// Union every region, clamped to the surface; regions that do not
/// intersect the surface (or are internally invalid) are skipped so a
/// fully-outside region can never fabricate edge damage.
fn union_clamped(
    regions: &[InclusiveRectangle],
    surface: (u16, u16),
) -> Option<InclusiveRectangle> {
    if surface.0 == 0 || surface.1 == 0 {
        return None;
    }
    let (max_x, max_y) = (surface.0 - 1, surface.1 - 1);
    let mut bounding: Option<InclusiveRectangle> = None;
    for region in regions {
        if region.left > max_x
            || region.top > max_y
            || region.right < region.left
            || region.bottom < region.top
        {
            continue;
        }
        let clamped = InclusiveRectangle {
            left: region.left,
            top: region.top,
            right: region.right.min(max_x),
            bottom: region.bottom.min(max_y),
        };
        bounding = Some(match bounding {
            None => clamped,
            Some(b) => InclusiveRectangle {
                left: b.left.min(clamped.left),
                top: b.top.min(clamped.top),
                right: b.right.max(clamped.right),
                bottom: b.bottom.max(clamped.bottom),
            },
        });
    }
    bounding
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: u16, top: u16, right: u16, bottom: u16) -> InclusiveRectangle {
        InclusiveRectangle {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn disjoint_regions_merge_into_one_bounding_rectangle() {
        let plan = plan_batch(
            &[rect(0, 0, 7, 3), rect(40, 50, 47, 55)],
            (64, 64),
            Some((64, 64)),
        )
        .expect("two disjoint regions yield a plan");
        assert!(
            !plan.full_refresh,
            "partial damage on an already-delivered surface stays kind 2"
        );
        assert_eq!(plan.rect, rect(0, 0, 47, 55));
    }

    #[test]
    fn overlapping_regions_union_without_gaps() {
        let plan = plan_batch(
            &[rect(10, 10, 30, 30), rect(20, 20, 40, 40)],
            (64, 64),
            Some((64, 64)),
        )
        .expect("overlapping regions yield a plan");
        assert!(!plan.full_refresh);
        assert_eq!(plan.rect, rect(10, 10, 40, 40));
    }

    #[test]
    fn single_region_passes_through() {
        let plan = plan_batch(&[rect(16, 16, 47, 47)], (64, 64), Some((64, 64)))
            .expect("single region yields a plan");
        assert!(!plan.full_refresh);
        assert_eq!(plan.rect, rect(16, 16, 47, 47));
    }

    #[test]
    fn initial_delivery_chooses_full_refresh() {
        let plan = plan_batch(&[rect(16, 16, 47, 47)], (64, 64), None)
            .expect("first delivery yields a plan");
        assert!(
            plan.full_refresh,
            "initial frame after (re)connect/surface creation is kind 1"
        );
    }

    #[test]
    fn resize_chooses_full_refresh() {
        let plan = plan_batch(&[rect(0, 0, 47, 31)], (48, 32), Some((64, 64)))
            .expect("dims change yields a plan");
        assert!(
            plan.full_refresh,
            "delivered dims differing from the decoded image is a resize: kind 1"
        );
    }

    #[test]
    fn full_surface_invalidation_is_full_refresh_even_after_initial() {
        let plan = plan_batch(&[rect(0, 0, 63, 63)], (64, 64), Some((64, 64)))
            .expect("full-surface damage yields a plan");
        assert!(
            plan.full_refresh,
            "full-surface invalidation (post-reset widen_to_full_refresh) is kind 1"
        );
    }

    #[test]
    fn regions_clamp_to_the_surface() {
        let plan = plan_batch(&[rect(60, 60, 70, 70)], (64, 64), Some((64, 64)))
            .expect("overhanging region still intersects");
        assert_eq!(plan.rect, rect(60, 60, 63, 63));
    }

    #[test]
    fn fully_outside_or_empty_regions_yield_no_plan() {
        assert!(plan_batch(&[], (64, 64), None).is_none());
        // A region entirely beyond the surface must not fabricate edge
        // damage (a naive clamp would invent a 1-px sliver).
        assert!(plan_batch(&[rect(70, 70, 79, 79)], (64, 64), Some((64, 64))).is_none());
        assert!(plan_batch(&[rect(0, 100, 7, 107)], (64, 64), None).is_none());
    }

    #[test]
    fn covers_full_surface_matches_exact_coverage_only() {
        assert!(covers_full_surface(rect(0, 0, 63, 63), (64, 64)));
        assert!(!covers_full_surface(rect(0, 0, 63, 62), (64, 64)));
        assert!(!covers_full_surface(rect(1, 0, 63, 63), (64, 64)));
    }
}
