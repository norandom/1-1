pub mod baseline;
pub mod client;
pub mod clipboard;
pub mod codec_policy;
pub mod display;
/// Task 9.2 (Req 4.1/4.2/9.2/9.4): EGFX adapter — registers the upstream
/// `GraphicsPipelineClient` on the DRDYNVC DVC and connects the upstream
/// compositor drain to the existing SharedFrame + packet frame paths.
pub mod egfx;
/// Task 5.4 (Req 12.4/12.10): build-gated frame-kind spy. Compiled ONLY in
/// `test-support` builds; production builds do not contain this module, its
/// environment variable, its events or its commands at all.
#[cfg(feature = "test-support")]
pub mod frame_spy;
pub mod frame_transport;
pub mod input;
pub mod session;
