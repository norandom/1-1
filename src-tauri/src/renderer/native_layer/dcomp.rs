//! The native layer on Windows (spec rdp-native-renderer, amendment 5a).
//!
//! A DirectComposition target on the main window, created with
//! `topmost = false`: DWM composes it below the window's child windows, and
//! WebView2's windowed controller is one of them. With the window
//! transparent, the page's transparent session area shows the target's
//! visual; page overlays draw above it and input stays with WebView2. The
//! visual covers the client area, and the render thread clears it to the
//! page's background and draws the visible session into its destination
//! rectangle, as on Wayland. Approach and numbers: `probes/windows-dcomp`.
//!
//! DirectComposition objects must not be used concurrently. They are created
//! on the main thread and afterwards used only through `Composition::apply`,
//! which serializes every call that reaches them (wgpu's `configure` calls
//! `SetContent`) and commits, since wgpu never does.

use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use tauri::{PhysicalSize, WindowEvent};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice2, IDCompositionDesktopDevice, IDCompositionTarget,
    IDCompositionVisual2,
};
use windows::core::{IUnknown, Interface};

use super::{Geometry, LayerTarget, SurfaceSource, Wake};

struct Objects {
    device: IDCompositionDesktopDevice,
    _target: IDCompositionTarget,
    visual: IDCompositionVisual2,
}

/// The window's DirectComposition objects behind one lock.
pub struct Composition {
    objects: Mutex<Objects>,
    /// The root visual, for wgpu's `SurfaceTargetUnsafe::CompositionVisual`.
    visual: *mut c_void,
}

// SAFETY: DirectComposition objects are not bound to a COM apartment; they
// must only not be used concurrently, and every use after creation goes
// through `objects`' lock. `visual` is a pointer to an object `objects`
// keeps alive, handed only to wgpu, which takes its own reference.
unsafe impl Send for Composition {}
unsafe impl Sync for Composition {}

impl Composition {
    /// The root visual; valid while `self` lives.
    pub fn visual(&self) -> *mut c_void {
        self.visual
    }

    /// Runs `f`, a wgpu call that reaches the visual, under the lock, then
    /// commits so its result shows.
    pub fn apply<T>(&self, f: impl FnOnce() -> T) -> T {
        let objects = self.objects.lock().unwrap();
        let result = f();
        // SAFETY: the device is used under its lock only.
        if let Err(error) = unsafe { objects.device.Commit() } {
            log::warn!("DirectComposition commit failed: {error}");
        }
        result
    }
}

/// Creates the layer below the window's web view. Runs on the main thread
/// (Tauri's `setup`), before the page loads.
pub fn attach(window: &tauri::WebviewWindow, wake: Arc<Wake>) -> Result<LayerTarget, String> {
    let hwnd = HWND(window.hwnd().map_err(|e| e.to_string())?.0);
    let create = || -> windows::core::Result<Objects> {
        // SAFETY: plain DirectComposition calls on the window's thread;
        // `hwnd` is the live main window, which outlives the layer.
        unsafe {
            let device: IDCompositionDesktopDevice = DCompositionCreateDevice2(None::<&IUnknown>)?;
            // topmost = false: composed below the window's child windows.
            let target = device.CreateTargetForHwnd(hwnd, false)?;
            let visual = device.CreateVisual()?;
            target.SetRoot(&visual)?;
            device.Commit()?;
            Ok(Objects {
                device,
                _target: target,
                visual,
            })
        }
    };
    let objects = create().map_err(|error| format!("composition: {error}"))?;
    let composition = Composition {
        visual: objects.visual.as_raw(),
        objects: Mutex::new(objects),
    };

    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let geometry = Arc::new(Mutex::new(client_geometry(size, scale)));
    // The visual sits at the client area's origin, where WebView2 is; the
    // render thread follows the client size and the monitor's scale.
    let follow = {
        let geometry = Arc::clone(&geometry);
        let wake = Arc::clone(&wake);
        let scale = Mutex::new(scale);
        move |event: &WindowEvent| {
            let next = match event {
                WindowEvent::Resized(size) => client_geometry(*size, *scale.lock().unwrap()),
                WindowEvent::ScaleFactorChanged {
                    scale_factor,
                    new_inner_size,
                    ..
                } => {
                    *scale.lock().unwrap() = *scale_factor;
                    client_geometry(*new_inner_size, *scale_factor)
                }
                _ => return,
            };
            let mut current = geometry.lock().unwrap();
            if *current != next {
                *current = next;
                drop(current);
                wake.notify();
            }
        }
    };
    window.on_window_event(follow);
    log::info!(
        "Native layer: DirectComposition visual below WebView2, {}x{} at scale {scale}",
        size.width,
        size.height
    );
    Ok(LayerTarget {
        surface: SurfaceSource::Composition(Arc::new(composition)),
        geometry,
        wake,
    })
}

/// The layer's geometry for a client area in physical pixels. A minimized
/// window reports 0×0, which the render thread treats as nothing to draw.
fn client_geometry(size: PhysicalSize<u32>, scale: f64) -> Geometry {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let logical = |value: u32| (f64::from(value) / scale).round() as u32;
    Geometry {
        logical_width: logical(size.width),
        logical_height: logical(size.height),
        buffer_width: size.width,
        buffer_height: size.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_area_in_physical_and_css_pixels() {
        let at = |w, h, scale| client_geometry(PhysicalSize::new(w, h), scale);
        assert_eq!(
            at(2400, 1500, 1.5),
            Geometry {
                logical_width: 1600,
                logical_height: 1000,
                buffer_width: 2400,
                buffer_height: 1500,
            }
        );
        assert_eq!(at(1600, 1000, 1.0).logical_width, 1600);
        // 125%: CSS pixels rounded, the buffer stays exact.
        let odd = at(1001, 801, 1.25);
        assert_eq!((odd.logical_width, odd.buffer_width), (801, 1001));
        // Minimized, and a nonsense scale.
        assert_eq!(at(0, 0, 1.5).buffer_width, 0);
        assert_eq!(at(800, 600, 0.0).logical_width, 800);
    }
}
