//! Which path shows the remote picture: the native GPU layer or the web
//! canvas (spec rdp-native-renderer, requirement 1).
//!
//! The choice is a pure function of facts collected at startup, so every
//! fallback reason is tested without a GPU or a window. The canvas is always
//! possible; the native layer only where every fact allows it.

use std::fmt;

use serde::Serialize;

/// The environment variable that overrides the automatic choice.
pub const RENDERER_ENV: &str = "ONE_ONE_RENDERER";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Path {
    Native,
    Canvas,
}

/// `ONE_ONE_RENDERER`: unset chooses automatically, `canvas` forces the
/// canvas, `native` tries the native layer and falls back only on failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Override {
    Auto,
    Canvas,
    Native,
}

impl Override {
    /// Parses the variable; an unknown value is ignored with a warning.
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            None | Some("") | Some("auto") => Self::Auto,
            Some("canvas") => Self::Canvas,
            Some("native") => Self::Native,
            Some(other) => {
                log::warn!(
                    "{RENDERER_ENV}={other} is not canvas, native or auto; choosing automatically"
                );
                Self::Auto
            }
        }
    }

    pub fn from_env() -> Self {
        Self::parse(std::env::var(RENDERER_ENV).ok().as_deref())
    }
}

/// The kind of display GDK opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Wayland,
    /// An X11 session, or XWayland through `GDK_BACKEND=x11`.
    X11,
    Other,
}

/// Why the GPU part could not be set up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuFailure {
    NoAdapter,
}

/// What startup learned. `gpu` is `None` while the GPU has not been tried,
/// which happens only when an earlier fact already rules the layer out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub requested: Override,
    pub linux: bool,
    pub display: Display,
    /// The first required Wayland global the compositor does not advertise.
    pub missing_interface: Option<&'static str>,
    /// Automatic mode may pick the native layer. Off until the reference
    /// measurement (spec task 7.3) passes; `ONE_ONE_RENDERER=native` opts in.
    pub auto_native: bool,
    pub gpu: Option<Result<(), GpuFailure>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    Available,
    ForcedNative,
    ForcedCanvas,
    NotLinux,
    X11Display,
    OtherDisplay,
    MissingWaylandInterface(&'static str),
    /// Everything allows the layer, but automatic mode does not choose it yet.
    OptInOnly,
    NoAdapter,
    SurfaceFailed(String),
    /// Set when the layer fails while sessions run (requirement 1.3).
    #[expect(dead_code, reason = "runtime fallback, spec task 6.1")]
    RuntimeFailure(String),
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Available => write!(f, "native layer available"),
            Self::ForcedNative => write!(f, "native layer requested by {RENDERER_ENV}"),
            Self::ForcedCanvas => write!(f, "canvas requested by {RENDERER_ENV}"),
            Self::NotLinux => write!(f, "the native layer exists only on Linux"),
            Self::X11Display => write!(f, "X11 or XWayland display"),
            Self::OtherDisplay => write!(f, "not a Wayland display"),
            Self::MissingWaylandInterface(name) => write!(f, "the compositor lacks {name}"),
            Self::OptInOnly => write!(
                f,
                "the native layer is opt-in for now ({RENDERER_ENV}=native)"
            ),
            Self::NoAdapter => write!(f, "no usable GPU adapter"),
            Self::SurfaceFailed(error) => write!(f, "GPU surface failed: {error}"),
            Self::RuntimeFailure(error) => write!(f, "native layer failed: {error}"),
        }
    }
}

/// Whether the GPU should be tried at all: everything else allows the layer.
pub fn worth_trying_gpu(facts: &Facts) -> bool {
    precondition(facts).is_none()
}

/// Facts that rule the layer out before the GPU is touched.
fn precondition(facts: &Facts) -> Option<Reason> {
    if facts.requested == Override::Canvas {
        return Some(Reason::ForcedCanvas);
    }
    if !facts.linux {
        return Some(Reason::NotLinux);
    }
    match facts.display {
        Display::Wayland => {}
        Display::X11 => return Some(Reason::X11Display),
        Display::Other => return Some(Reason::OtherDisplay),
    }
    if let Some(name) = facts.missing_interface {
        return Some(Reason::MissingWaylandInterface(name));
    }
    (facts.requested == Override::Auto && !facts.auto_native).then_some(Reason::OptInOnly)
}

/// The display path for these facts, and why.
pub fn choose(facts: &Facts) -> (Path, Reason) {
    if let Some(reason) = precondition(facts) {
        return (Path::Canvas, reason);
    }
    match &facts.gpu {
        Some(Ok(())) if facts.requested == Override::Native => (Path::Native, Reason::ForcedNative),
        Some(Ok(())) => (Path::Native, Reason::Available),
        Some(Err(GpuFailure::NoAdapter)) => (Path::Canvas, Reason::NoAdapter),
        None => (
            Path::Canvas,
            Reason::SurfaceFailed("the GPU was not tried".into()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wayland() -> Facts {
        Facts {
            requested: Override::Auto,
            linux: true,
            display: Display::Wayland,
            missing_interface: None,
            auto_native: true,
            gpu: Some(Ok(())),
        }
    }

    #[test]
    fn a_wayland_session_with_a_gpu_gets_the_native_layer() {
        assert_eq!(choose(&wayland()), (Path::Native, Reason::Available));
        assert!(worth_trying_gpu(&wayland()));
    }

    #[test]
    fn every_fallback_reason_chooses_the_canvas() {
        let cases = [
            (
                Facts {
                    requested: Override::Canvas,
                    ..wayland()
                },
                Reason::ForcedCanvas,
            ),
            (
                Facts {
                    linux: false,
                    ..wayland()
                },
                Reason::NotLinux,
            ),
            (
                Facts {
                    display: Display::X11,
                    ..wayland()
                },
                Reason::X11Display,
            ),
            (
                Facts {
                    display: Display::Other,
                    ..wayland()
                },
                Reason::OtherDisplay,
            ),
            (
                Facts {
                    missing_interface: Some("wl_subcompositor"),
                    ..wayland()
                },
                Reason::MissingWaylandInterface("wl_subcompositor"),
            ),
            (
                Facts {
                    auto_native: false,
                    ..wayland()
                },
                Reason::OptInOnly,
            ),
            (
                Facts {
                    gpu: Some(Err(GpuFailure::NoAdapter)),
                    ..wayland()
                },
                Reason::NoAdapter,
            ),
            (
                Facts {
                    gpu: None,
                    ..wayland()
                },
                Reason::SurfaceFailed("the GPU was not tried".into()),
            ),
        ];
        for (facts, reason) in cases {
            assert_eq!(choose(&facts), (Path::Canvas, reason), "{facts:?}");
        }
    }

    #[test]
    fn the_gpu_is_not_tried_when_an_earlier_fact_rules_the_layer_out() {
        for facts in [
            Facts {
                requested: Override::Canvas,
                ..wayland()
            },
            Facts {
                linux: false,
                ..wayland()
            },
            Facts {
                display: Display::X11,
                ..wayland()
            },
            Facts {
                missing_interface: Some("wl_compositor"),
                ..wayland()
            },
            Facts {
                auto_native: false,
                ..wayland()
            },
        ] {
            assert!(!worth_trying_gpu(&facts), "{facts:?}");
        }
    }

    #[test]
    fn forcing_native_still_falls_back_on_failure() {
        let forced = Facts {
            requested: Override::Native,
            auto_native: false,
            ..wayland()
        };
        assert_eq!(choose(&forced), (Path::Native, Reason::ForcedNative));
        assert_eq!(
            choose(&Facts {
                display: Display::X11,
                ..forced.clone()
            }),
            (Path::Canvas, Reason::X11Display)
        );
        assert_eq!(
            choose(&Facts {
                gpu: Some(Err(GpuFailure::NoAdapter)),
                ..forced
            }),
            (Path::Canvas, Reason::NoAdapter)
        );
    }

    #[test]
    fn the_override_variable() {
        assert_eq!(Override::parse(None), Override::Auto);
        assert_eq!(Override::parse(Some("")), Override::Auto);
        assert_eq!(Override::parse(Some("auto")), Override::Auto);
        assert_eq!(Override::parse(Some("canvas")), Override::Canvas);
        assert_eq!(Override::parse(Some(" Native ")), Override::Native);
        assert_eq!(Override::parse(Some("gpu")), Override::Auto);
    }

    #[test]
    fn reasons_read_as_text_for_the_hud_and_log() {
        assert_eq!(Reason::X11Display.to_string(), "X11 or XWayland display");
        assert_eq!(
            Reason::MissingWaylandInterface("wl_subcompositor").to_string(),
            "the compositor lacks wl_subcompositor"
        );
        assert_eq!(
            serde_json::to_value(Path::Native).unwrap(),
            serde_json::json!("native")
        );
    }
}

/// The path in force, shared with the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayPathInfo {
    pub path: Path,
    pub reason: String,
}

/// App state: the path chosen at startup, changed only by a runtime fallback.
pub struct DisplayPathState(std::sync::Mutex<(Path, Reason)>);

/// Emitted when the path changes while the app runs.
#[expect(dead_code, reason = "runtime fallback, spec task 6.1")]
pub const DISPLAY_PATH_EVENT: &str = "display-path";

impl DisplayPathState {
    pub fn new(path: Path, reason: Reason) -> Self {
        Self(std::sync::Mutex::new((path, reason)))
    }

    pub fn info(&self) -> DisplayPathInfo {
        let guard = self.0.lock().unwrap();
        DisplayPathInfo {
            path: guard.0,
            reason: guard.1.to_string(),
        }
    }

    pub fn path(&self) -> Path {
        self.0.lock().unwrap().0
    }

    /// Records a fallback after startup (the layer could not be built, or
    /// failed while running).
    pub fn fall_back(&self, reason: Reason) {
        *self.0.lock().unwrap() = (Path::Canvas, reason);
    }
}

#[tauri::command]
pub fn get_display_path(state: tauri::State<'_, DisplayPathState>) -> DisplayPathInfo {
    state.info()
}

/// Collects the startup facts and chooses the path. Runs before the main
/// window exists: the window is transparent only on the native path.
pub fn decide() -> (Path, Reason) {
    let mut facts = Facts {
        requested: Override::from_env(),
        linux: cfg!(target_os = "linux"),
        display: Display::Other,
        missing_interface: None,
        auto_native: false,
        gpu: None,
    };
    #[cfg(target_os = "linux")]
    linux::collect(&mut facts);
    if worth_trying_gpu(&facts) {
        facts.gpu = Some(try_gpu());
    }
    let (path, reason) = choose(&facts);
    log::info!("Remote picture: {path:?} path ({reason})");
    (path, reason)
}

/// An adapter exists. The surface itself is checked when the layer is built
/// (spec task 3.1); a failure there falls back like a runtime failure.
fn try_gpu() -> Result<(), GpuFailure> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
        ..Default::default()
    });
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        ..Default::default()
    }))
    .map(drop)
    .ok_or(GpuFailure::NoAdapter)
}

#[cfg(target_os = "linux")]
mod linux {
    use gtk::glib::translate::ToGlibPtr;
    use gtk::prelude::*;
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_registry::WlRegistry;
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};

    use super::{Display, Facts};

    /// Globals the layer needs from the compositor.
    const REQUIRED: [&str; 2] = ["wl_compositor", "wl_subcompositor"];

    struct Registry;
    impl Dispatch<WlRegistry, GlobalListContents> for Registry {
        fn event(
            _: &mut Self,
            _: &WlRegistry,
            _: <WlRegistry as Proxy>::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    pub(super) fn collect(facts: &mut Facts) {
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        let kind = display.type_().name();
        facts.display = if kind.contains("Wayland") {
            Display::Wayland
        } else if kind.contains("X11") {
            Display::X11
        } else {
            Display::Other
        };
        if facts.display != Display::Wayland {
            return;
        }
        // SAFETY: GDK owns the wl_display for the app's lifetime; the foreign
        // connection only adds our own event queue to it.
        let conn = unsafe {
            let ptr: *mut gtk::gdk::ffi::GdkDisplay = display.to_glib_none().0;
            let wl_display = gdk_wayland_sys::gdk_wayland_display_get_wl_display(ptr as *mut _);
            Connection::from_backend(wayland_backend::client::Backend::from_foreign_display(
                wl_display as *mut _,
            ))
        };
        let Ok((globals, _queue)) = registry_queue_init::<Registry>(&conn) else {
            facts.missing_interface = Some("wl_registry");
            return;
        };
        let advertised = globals.contents().clone_list();
        facts.missing_interface = REQUIRED
            .into_iter()
            .find(|name| !advertised.iter().any(|global| global.interface == *name));
    }
}
