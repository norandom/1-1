//! The native layer's Wayland surface (spec rdp-native-renderer, tasks 3.1
//! and 3.3).
//!
//! A surface of our own on GDK's Wayland connection, made a desynchronized
//! subsurface placed below GTK's toplevel surface. It covers the web view;
//! the render thread clears it to the app's background and draws the visible
//! session where the page leaves a transparent hole. Input stays with the web
//! view (empty input region). The approach and its numbers come from
//! `probes/native-subsurface`.
//!
//! GTK objects are not `Send`: the layer lives on GTK's main thread in a
//! thread-local, and the render thread gets only raw handles, the geometry
//! and a wake-up signal.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Size of the layer: what the render thread configures its surface to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Geometry {
    /// Web view size in logical (CSS) pixels.
    pub logical_width: u32,
    pub logical_height: u32,
    /// Buffer size in device pixels.
    pub buffer_width: u32,
    pub buffer_height: u32,
}

impl Geometry {
    /// Device pixels per logical pixel.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used by the render thread, spec task 4.1")
    )]
    pub fn scale(&self) -> f64 {
        if self.logical_width == 0 {
            1.0
        } else {
            f64::from(self.buffer_width) / f64::from(self.logical_width)
        }
    }
}

/// How the layer reaches device pixels for a logical size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scaling {
    /// `wl_surface.set_buffer_scale` with GDK's integer scale.
    Integer(u32),
    /// `wp_fractional_scale_v1`'s preferred scale (in 120ths) with a
    /// `wp_viewporter` destination at the logical size.
    Fractional(u32),
}

impl Scaling {
    pub fn buffer_size(self, logical: (u32, u32)) -> (u32, u32) {
        let scaled = |value: u32| match self {
            Self::Integer(scale) => value * scale.max(1),
            Self::Fractional(scale) => (u64::from(value) * u64::from(scale)).div_ceil(120) as u32,
        };
        (scaled(logical.0).max(1), scaled(logical.1).max(1))
    }
}

/// Wakes the render thread: geometry, view or frame changes.
#[derive(Default)]
pub struct Wake {
    generation: Mutex<u64>,
    signal: Condvar,
}

impl Wake {
    pub fn notify(&self) {
        *self.generation.lock().unwrap() += 1;
        self.signal.notify_all();
    }

    /// Waits until something changed after `seen`, or the timeout; returns
    /// the current generation.
    pub fn wait(&self, seen: u64, timeout: Duration) -> u64 {
        let guard = self.generation.lock().unwrap();
        let (guard, _) = self
            .signal
            .wait_timeout_while(guard, timeout, |generation| *generation == seen)
            .unwrap();
        *guard
    }
}

/// Where the render thread's surface comes from.
#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
pub enum SurfaceSource {
    /// Wayland: our subsurface on GDK's connection.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code, reason = "Wayland only"))]
    Raw(RawDisplayHandle, RawWindowHandle),
    /// Windows: the layer's DirectComposition visual (amendment 5a).
    #[cfg(windows)]
    Composition(Arc<dcomp::Composition>),
}

/// What the render thread needs to draw into the layer.
#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
pub struct LayerTarget {
    pub surface: SurfaceSource,
    pub geometry: Arc<Mutex<Geometry>>,
    pub wake: Arc<Wake>,
}

// SAFETY: the raw handles name libwayland objects owned by the layer for the
// window's lifetime, and libwayland proxies may be used from any thread; the
// Windows composition is `Send` on its own.
unsafe impl Send for LayerTarget {}

#[cfg(target_os = "linux")]
pub use linux::attach;

#[cfg(windows)]
pub mod dcomp;
#[cfg(windows)]
pub use dcomp::attach;

#[cfg(target_os = "linux")]
mod linux {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::ptr::NonNull;
    use std::sync::{Arc, Mutex};

    use gtk::glib;
    use gtk::glib::translate::ToGlibPtr;
    use gtk::prelude::*;
    use raw_window_handle::{
        RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
    };
    use wayland_backend::client::{Backend, ObjectId};
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{
        wl_compositor::WlCompositor, wl_region::WlRegion, wl_registry::WlRegistry,
        wl_subcompositor::WlSubcompositor, wl_subsurface::WlSubsurface, wl_surface::WlSurface,
    };
    use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
    use wayland_protocols::wp::fractional_scale::v1::client::{
        wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        wp_fractional_scale_v1::{self, WpFractionalScaleV1},
    };
    use wayland_protocols::wp::viewporter::client::{
        wp_viewport::WpViewport, wp_viewporter::WpViewporter,
    };

    use super::{Geometry, LayerTarget, Scaling, SurfaceSource, Wake};

    /// Events for our queue: only the preferred fractional scale matters.
    #[derive(Default)]
    struct State {
        preferred: Option<u32>,
    }

    impl Dispatch<WlRegistry, GlobalListContents> for State {
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

    impl Dispatch<WpFractionalScaleV1, ()> for State {
        fn event(
            state: &mut Self,
            _: &WpFractionalScaleV1,
            event: wp_fractional_scale_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
                state.preferred = Some(scale);
            }
        }
    }

    delegate_noop!(State: WlCompositor);
    delegate_noop!(State: WlSubcompositor);
    delegate_noop!(State: WlSubsurface);
    delegate_noop!(State: WlRegion);
    delegate_noop!(State: ignore WlSurface);
    delegate_noop!(State: WpFractionalScaleManagerV1);
    delegate_noop!(State: WpViewporter);
    delegate_noop!(State: WpViewport);

    struct Layer {
        conn: Connection,
        queue: EventQueue<State>,
        state: State,
        gtk_window: gtk::ApplicationWindow,
        webview: gtk::Widget,
        child: WlSurface,
        subsurface: WlSubsurface,
        viewport: Option<WpViewport>,
        _fractional: Option<WpFractionalScaleV1>,
        geometry: Arc<Mutex<Geometry>>,
        wake: Arc<Wake>,
        position: (i32, i32),
    }

    thread_local! {
        static LAYER: RefCell<Option<Layer>> = const { RefCell::new(None) };
    }

    /// Creates the layer below the window's web view. Runs on GTK's main
    /// thread (Tauri's `setup`).
    pub fn attach(window: &tauri::WebviewWindow, wake: Arc<Wake>) -> Result<LayerTarget, String> {
        let gtk_window = window.gtk_window().map_err(|e| e.to_string())?;
        let webview =
            find_webview(gtk_window.upcast_ref()).ok_or("no WebKitWebView in the window")?;
        gtk_window.realize();
        let gdk_window = gtk_window.window().ok_or("the window has no GdkWindow")?;
        let display = gdk_window.display();
        // SAFETY: GDK owns these objects for the window's lifetime.
        let (wl_display, wl_surface) = unsafe {
            let display_ptr: *mut gtk::gdk::ffi::GdkDisplay = display.to_glib_none().0;
            let window_ptr: *mut gtk::gdk::ffi::GdkWindow = gdk_window.to_glib_none().0;
            (
                gdk_wayland_sys::gdk_wayland_display_get_wl_display(display_ptr as *mut _),
                gdk_wayland_sys::gdk_wayland_window_get_wl_surface(window_ptr as *mut _),
            )
        };
        if wl_display.is_null() || wl_surface.is_null() {
            return Err("GDK gave no Wayland display or surface".into());
        }
        // SAFETY: as in the probe; a foreign connection adds our own queue.
        let conn = Connection::from_backend(unsafe {
            Backend::from_foreign_display(wl_display as *mut _)
        });
        let (globals, mut queue) =
            registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
        let qh = queue.handle();
        let compositor: WlCompositor = globals.bind(&qh, 4..=6, ()).map_err(|e| e.to_string())?;
        let subcompositor: WlSubcompositor =
            globals.bind(&qh, 1..=1, ()).map_err(|e| e.to_string())?;
        let parent = WlSurface::from_id(&conn, unsafe {
            ObjectId::from_ptr(WlSurface::interface(), wl_surface as *mut _)
                .map_err(|e| e.to_string())?
        })
        .map_err(|e| e.to_string())?;

        let child = compositor.create_surface(&qh, ());
        let empty = compositor.create_region(&qh, ());
        child.set_input_region(Some(&empty));
        empty.destroy();
        let subsurface = subcompositor.get_subsurface(&child, &parent, &qh, ());
        subsurface.place_below(&parent);
        subsurface.set_desync();

        // Exact pixels where the compositor supports it (task 3.3).
        let fractional = globals
            .bind::<WpFractionalScaleManagerV1, _, _>(&qh, 1..=1, ())
            .ok()
            .map(|manager| manager.get_fractional_scale(&child, &qh, ()));
        let viewport = fractional
            .as_ref()
            .and_then(|_| globals.bind::<WpViewporter, _, _>(&qh, 1..=1, ()).ok())
            .map(|viewporter| viewporter.get_viewport(&child, &qh, ()));
        child.commit();
        conn.flush().map_err(|e| e.to_string())?;
        let mut state = State::default();
        queue.roundtrip(&mut state).map_err(|e| e.to_string())?;

        let target = LayerTarget {
            surface: SurfaceSource::Raw(
                RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                    NonNull::new(wl_display).ok_or("null wl_display")?,
                )),
                RawWindowHandle::Wayland(WaylandWindowHandle::new(
                    NonNull::new(child.id().as_ptr() as *mut c_void).ok_or("null wl_surface")?,
                )),
            ),
            geometry: Arc::new(Mutex::new(Geometry::default())),
            wake,
        };
        log::info!(
            "Native layer below the web view ({} scaling)",
            if viewport.is_some() {
                "fractional"
            } else {
                "integer"
            }
        );
        let layer = Layer {
            conn,
            queue,
            state,
            gtk_window,
            webview: webview.clone(),
            child,
            subsurface,
            viewport: viewport.clone(),
            _fractional: fractional,
            geometry: Arc::clone(&target.geometry),
            wake: Arc::clone(&target.wake),
            position: (i32::MIN, i32::MIN),
        };
        LAYER.with(|slot| *slot.borrow_mut() = Some(layer));

        // Follow the web view's place and size, and the output's scale.
        webview.connect_size_allocate(|_, _| update());
        gdk_window.connect_notify_local(Some("scale-factor"), |_, _| update());
        glib::timeout_add_local(std::time::Duration::from_millis(500), || {
            update();
            glib::ControlFlow::Continue
        });
        update();
        Ok(target)
    }

    /// Reads pending scale events and applies the web view's current place
    /// and size. Cheap when nothing changed.
    fn update() {
        LAYER.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(layer) = slot.as_mut() else { return };
            let _ = layer.queue.dispatch_pending(&mut layer.state);

            let allocation = layer.webview.allocation();
            let position = layer
                .webview
                .translate_coordinates(&layer.gtk_window, 0, 0)
                .unwrap_or((0, 0));
            let logical = (
                allocation.width().max(1) as u32,
                allocation.height().max(1) as u32,
            );
            let scaling = match (layer.viewport.as_ref(), layer.state.preferred) {
                (Some(_), Some(scale)) => Scaling::Fractional(scale),
                _ => Scaling::Integer(layer.gtk_window.scale_factor().max(1) as u32),
            };
            let (buffer_width, buffer_height) = scaling.buffer_size(logical);
            let geometry = Geometry {
                logical_width: logical.0,
                logical_height: logical.1,
                buffer_width,
                buffer_height,
            };

            let mut changed = false;
            if position != layer.position {
                layer.position = position;
                layer.subsurface.set_position(position.0, position.1);
                // Subsurface position is parent state: GTK's next commit applies it.
                layer.gtk_window.queue_draw();
                changed = true;
            }
            let mut current = layer.geometry.lock().unwrap();
            if *current != geometry {
                match (&scaling, &layer.viewport) {
                    (Scaling::Fractional(_), Some(viewport)) => {
                        layer.child.set_buffer_scale(1);
                        viewport.set_destination(logical.0 as i32, logical.1 as i32);
                    }
                    (Scaling::Integer(scale), _) => layer.child.set_buffer_scale(*scale as i32),
                    _ => {}
                }
                *current = geometry;
                changed = true;
            }
            drop(current);
            if changed {
                let _ = layer.conn.flush();
                layer.wake.notify();
            }
        });
    }

    fn find_webview(widget: &gtk::Widget) -> Option<gtk::Widget> {
        if widget.is::<webkit2gtk::WebView>() {
            return Some(widget.clone());
        }
        let container = widget.downcast_ref::<gtk::Container>()?;
        container.children().iter().find_map(find_webview)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_scaling_multiplies() {
        assert_eq!(Scaling::Integer(2).buffer_size((1888, 1137)), (3776, 2274));
        assert_eq!(
            Scaling::Integer(0).buffer_size((10, 10)),
            (10, 10),
            "scale 0 is read as 1"
        );
    }

    #[test]
    fn fractional_scaling_uses_120ths_and_rounds_up() {
        // 1.6 = 192/120: the maintainer's display.
        assert_eq!(
            Scaling::Fractional(192).buffer_size((1888, 1137)),
            (3021, 1820)
        );
        assert_eq!(Scaling::Fractional(120).buffer_size((800, 600)), (800, 600));
        assert_eq!(Scaling::Fractional(150).buffer_size((0, 3)), (1, 4));
    }

    #[test]
    fn geometry_scale() {
        let g = Geometry {
            logical_width: 100,
            logical_height: 50,
            buffer_width: 160,
            buffer_height: 80,
        };
        assert!((g.scale() - 1.6).abs() < 1e-9);
        assert_eq!(Geometry::default().scale(), 1.0);
    }

    #[test]
    fn wake_returns_on_notify_or_timeout() {
        let wake = Arc::new(Wake::default());
        assert_eq!(
            wake.wait(0, Duration::from_millis(1)),
            0,
            "timeout without a change"
        );
        wake.notify();
        assert_eq!(
            wake.wait(0, Duration::from_secs(5)),
            1,
            "a change since 0 returns at once"
        );
        let waiter = {
            let wake = Arc::clone(&wake);
            std::thread::spawn(move || wake.wait(1, Duration::from_secs(5)))
        };
        std::thread::sleep(Duration::from_millis(20));
        wake.notify();
        assert_eq!(waiter.join().unwrap(), 2);
    }
}
