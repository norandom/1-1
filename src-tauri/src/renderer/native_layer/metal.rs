//! The native layer on macOS (spec rdp-native-renderer, task 9.2).
//!
//! A layer-hosting `NSView` with a `CAMetalLayer`, added to the WKWebView's
//! parent view below the web view. With the web view's `drawsBackground`
//! off, the page's transparent session area shows the layer; page overlays
//! draw above it and input stays with the web view, which covers the layer
//! completely. The window itself stays opaque. The render thread clears the
//! layer to the page's background and draws the visible session into its
//! destination rectangle, as on Wayland and Windows.
//!
//! AppKit objects are created here, on the main thread, and stay with the
//! view hierarchy. The render thread gets only the `CAMetalLayer`, which wgpu
//! sizes (`drawableSize`) and presents to.

use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use objc2::rc::Retained;
use objc2::runtime::AnyClass;
use objc2::{MainThreadMarker, Message, msg_send};
use objc2_app_kit::{NSView, NSWindowOrderingMode};
use objc2_foundation::{NSNumber, NSObjectProtocol, ns_string};
use objc2_quartz_core::CAMetalLayer;
use tauri::WindowEvent;

use super::{LayerTarget, SurfaceSource, Wake, client_geometry};

/// The layer below the web view, for wgpu's
/// `SurfaceTargetUnsafe::CoreAnimationLayer`.
pub struct Layer(Retained<CAMetalLayer>);

// SAFETY: after `attach`, the layer is used only by the render thread's wgpu
// surface, and `CAMetalLayer`'s drawable calls are made from render threads
// by design. The view that hosts it keeps it alive with the window.
unsafe impl Send for Layer {}

impl Layer {
    /// The `CAMetalLayer`; valid while `self` lives.
    pub fn as_ptr(&self) -> *mut c_void {
        Retained::as_ptr(&self.0) as *mut c_void
    }
}

/// Creates the layer below the window's web view. Runs on the main thread
/// (Tauri's `setup`), before the page loads.
pub fn attach(window: &tauri::WebviewWindow, wake: Arc<Wake>) -> Result<LayerTarget, String> {
    let mtm = MainThreadMarker::new().ok_or("the layer is built on the main thread only")?;
    let content = window.ns_view().map_err(|e| e.to_string())?;
    // SAFETY: the live main window's content view.
    let content: Retained<NSView> =
        unsafe { Retained::retain(content.cast()) }.ok_or("the window has no content view")?;
    let webview = find_webview(&content).ok_or("no WKWebView in the window")?;
    // SAFETY: plain AppKit calls on the main thread.
    let parent = unsafe { webview.superview() }.ok_or("the web view has no parent view")?;

    let layer = CAMetalLayer::new();
    layer.setOpaque(true);
    let view = NSView::initWithFrame(mtm.alloc(), webview.frame());
    // The layer before `wantsLayer`: a layer-hosting view, whose layer AppKit
    // resizes with the view and never draws into.
    view.setLayer(Some(&layer));
    view.setWantsLayer(true);
    view.setAutoresizingMask(webview.autoresizingMask());
    parent.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, Some(&webview));

    // What `transparent` does in wry, without a transparent window: the web
    // view stops painting its own background; the page still paints its own.
    let no = NSNumber::new_bool(false);
    // SAFETY: key-value coding on the web view; WKWebView accepts this key.
    unsafe {
        let _: () = msg_send![&*webview, setValue: &*no, forKey: ns_string!("drawsBackground")];
    }

    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let geometry = Arc::new(Mutex::new(client_geometry(size, scale)));
    // The web view fills the content view (the title bar is an overlay); the
    // render thread follows the content size and the display's scale.
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
        "Native layer: CAMetalLayer below WKWebView, {}x{} at scale {scale}",
        size.width,
        size.height
    );
    Ok(LayerTarget {
        surface: SurfaceSource::Metal(Layer(layer)),
        geometry,
        wake,
    })
}

fn find_webview(view: &NSView) -> Option<Retained<NSView>> {
    let class = AnyClass::get(c"WKWebView")?;
    if view.isKindOfClass(class) {
        return Some(view.retain());
    }
    view.subviews()
        .iter()
        .find_map(|child| find_webview(&child))
}
