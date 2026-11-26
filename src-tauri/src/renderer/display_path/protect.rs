//! Protection tests for `display_path.rs`, written from planted faults.
//!
//! Each test was shown to fail on a small mistake planted in the code and to
//! pass on the code as it is (spec protect-display-path). They fix what the
//! functions do today, so that a later reduction cannot change it unseen.

use super::*;

/// `Platform::current` names the platform this build is for.
#[test]
fn the_platform_is_the_one_this_build_is_for() {
    #[cfg(windows)]
    assert_eq!(Platform::current(), Platform::Windows);
    #[cfg(target_os = "linux")]
    assert_eq!(Platform::current(), Platform::Linux);
    #[cfg(not(any(windows, target_os = "linux")))]
    assert_eq!(Platform::current(), Platform::Other);
}

/// After a fallback the state names the canvas and the reason given.
#[test]
fn a_fallback_moves_the_state_to_the_canvas_with_its_reason() {
    let state = DisplayPathState::new(Path::Native, Reason::Available);
    state.fall_back(Reason::RuntimeFailure("lost device".into()));
    assert_eq!(state.path(), Path::Canvas);
    assert_eq!(state.info().reason, "native layer failed: lost device");
}

/// DX12 for a DirectComposition visual on Windows; Vulkan and GL elsewhere.
#[test]
fn the_layer_asks_for_the_backends_of_this_platform() {
    #[cfg(windows)]
    assert_eq!(layer_backends(), wgpu::Backends::DX12);
    #[cfg(not(windows))]
    assert_eq!(
        layer_backends(),
        wgpu::Backends::VULKAN | wgpu::Backends::GL
    );
}

/// On Windows nothing rules the layer out when it is asked for, so `decide`
/// tries the GPU: the answer is the layer, or that no adapter was found. It
/// asks the real GPU, and needs `ONE_ONE_RENDERER` unset.
#[cfg(windows)]
#[test]
fn decide_tries_the_gpu_when_nothing_rules_the_layer_out() {
    assert!(
        std::env::var_os(RENDERER_ENV).is_none(),
        "unset {RENDERER_ENV} to run this test"
    );
    let answer = decide("native");
    assert!(
        answer == (Path::Native, Reason::ForcedNative)
            || answer == (Path::Canvas, Reason::NoAdapter),
        "decide answered without trying the GPU: {answer:?}"
    );
}

/// After a fallback the layer takes no store, and holds none it took before:
/// a session created while the layer failed is sent to the canvas.
#[test]
fn after_a_fallback_the_layer_takes_no_store() {
    let state = DisplayPathState::new(Path::Native, Reason::Available);
    let before = crate::renderer::shared_frame::SharedFrame::new(1, 1);
    assert!(state.register("a", &before));
    state.fall_back(Reason::RuntimeFailure("lost device".into()));
    assert!(state.get("a").is_none());
    let after = crate::renderer::shared_frame::SharedFrame::new(1, 1);
    assert!(!state.register("b", &after));
    assert!(state.get("b").is_none());
}
