pub mod baseline;
pub mod client;
pub mod clipboard;
pub mod codec_policy;
pub mod display;
/// Task 9.2 (Req 4.1/4.2/9.2/9.4): EGFX adapter — registers the upstream
/// `GraphicsPipelineClient` on the DRDYNVC DVC and connects the upstream
/// compositor drain to the existing SharedFrame + packet frame paths.
pub mod egfx;
/// Task 9.4 (Req 4.1/4.4/9.8): pure dirty-region batching math for the
/// session drain — merge one drained batch's regions into a single
/// bounding rectangle and choose the full-frame vs dirty-rect packet kind.
pub mod frame_batch;
pub mod frame_delivery;
/// Task 5.4 (Req 12.4/12.10): build-gated frame-kind spy. Compiled ONLY in
/// `test-support` builds; production builds do not contain this module, its
/// environment variable, its events or its commands at all.
#[cfg(feature = "test-support")]
pub mod frame_spy;
pub mod frame_transport;
pub mod input;
pub mod session;
