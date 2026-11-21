//! Spike: draw remote frames below a transparent web view on Linux.
//!
//! PROBE_MODE=subsurface (default): a wgpu layer in a wl_subsurface placed
//! below GTK's toplevel surface; the page leaves a transparent hole for it.
//! PROBE_MODE=canvas: the same pattern drawn by the page into a canvas (the
//! current 1-1 path, without the IPC).
//! PROBE_UPLOAD=full (default) uploads the whole frame every time; strip
//! uploads 64 rows per frame, like a terminal scrolling.
//!
//! Prints one line a second: frames presented and the time spent per frame.

#![cfg_attr(not(target_os = "linux"), allow(unused))]

use std::sync::{Arc, Mutex};

use serde::Deserialize;
use tauri::Manager;

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
struct Hole {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

struct Probe {
    mode: String,
    hole: Arc<Mutex<Option<Hole>>>,
}

#[cfg(target_os = "linux")]
thread_local! {
    /// GTK objects stay on the main thread.
    static LAYER: std::cell::RefCell<Option<linux::Layer>> = const { std::cell::RefCell::new(None) };
}

#[tauri::command]
fn mode(probe: tauri::State<'_, Probe>) -> String {
    probe.mode.clone()
}

#[tauri::command]
fn fps(source: String, fps: u32) {
    println!("{source}: {fps} fps");
}

#[tauri::command]
fn hole(app: tauri::AppHandle, x: f64, y: f64, width: f64, height: f64) {
    let rect = Hole { x, y, width, height };
    let probe = app.state::<Probe>();
    *probe.hole.lock().unwrap() = Some(rect);
    #[cfg(target_os = "linux")]
    {
        let _ = app.run_on_main_thread(move || {
            LAYER.with(|layer| {
                if let Some(layer) = layer.borrow().as_ref() {
                    layer.place(rect);
                }
            });
        });
    }
}

fn main() {
    let probe_mode = std::env::var("PROBE_MODE").unwrap_or_else(|_| "subsurface".into());
    let hole_state = Arc::new(Mutex::new(None));
    tauri::Builder::default()
        .manage(Probe {
            mode: probe_mode.clone(),
            hole: Arc::clone(&hole_state),
        })
        .invoke_handler(tauri::generate_handler![mode, fps, hole])
        .setup(move |app| {
            #[cfg(target_os = "linux")]
            if probe_mode == "subsurface" {
                let window = app.get_webview_window("main").unwrap();
                let layer = linux::Layer::new(&window, Arc::clone(&hole_state))?;
                // A hole reported before the layer existed.
                if let Some(rect) = *hole_state.lock().unwrap() {
                    layer.place(rect);
                }
                LAYER.with(|slot| *slot.borrow_mut() = Some(layer));
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("probe failed");
}

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::c_void;
    use std::ptr::NonNull;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

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
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};

    use super::Hole;

    struct State;
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
    delegate_noop!(State: WlCompositor);
    delegate_noop!(State: WlSubcompositor);
    delegate_noop!(State: WlSubsurface);
    delegate_noop!(State: WlRegion);
    delegate_noop!(State: ignore WlSurface);

    pub struct Layer {
        conn: Connection,
        subsurface: WlSubsurface,
        gtk_window: gtk::ApplicationWindow,
        webview: gtk::Widget,
        // Keeps the Wayland objects alive for the window's lifetime.
        _child: WlSurface,
        _queue: wayland_client::EventQueue<State>,
    }

    impl Layer {
        pub fn new(
            window: &tauri::WebviewWindow,
            hole: Arc<Mutex<Option<Hole>>>,
        ) -> Result<Self, Box<dyn std::error::Error>> {
            let gtk_window = window.gtk_window()?;
            let webview = find_webview(gtk_window.upcast_ref()).ok_or("no WebKitWebView in the window")?;
            gtk_window.realize();
            let gdk_window = gtk_window.window().ok_or("no GdkWindow")?;
            let display = gdk_window.display();
            let backend_name = display.type_().name();
            if !backend_name.contains("Wayland") {
                return Err(format!("not a Wayland display ({backend_name}); run without GDK_BACKEND=x11").into());
            }
            let scale = gdk_window.scale_factor().max(1);
            let (wl_display, wl_surface) = unsafe {
                let display_ptr: *mut gtk::gdk::ffi::GdkDisplay = display.to_glib_none().0;
                let window_ptr: *mut gtk::gdk::ffi::GdkWindow = gdk_window.to_glib_none().0;
                (
                    gdk_wayland_sys::gdk_wayland_display_get_wl_display(display_ptr as *mut _),
                    gdk_wayland_sys::gdk_wayland_window_get_wl_surface(window_ptr as *mut _),
                )
            };
            let conn = Connection::from_backend(unsafe {
                Backend::from_foreign_display(wl_display as *mut _)
            });
            let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
            let qh = queue.handle();
            let compositor: WlCompositor = globals.bind(&qh, 4..=6, ())?;
            let subcompositor: WlSubcompositor = globals.bind(&qh, 1..=1, ())?;
            let parent = WlSurface::from_id(&conn, unsafe {
                ObjectId::from_ptr(WlSurface::interface(), wl_surface as *mut _)?
            })?;

            let child = compositor.create_surface(&qh, ());
            // Input stays with the web view above.
            let empty = compositor.create_region(&qh, ());
            child.set_input_region(Some(&empty));
            empty.destroy();
            child.set_buffer_scale(scale);
            let subsurface = subcompositor.get_subsurface(&child, &parent, &qh, ());
            subsurface.place_below(&parent);
            subsurface.set_desync();
            child.commit();
            conn.flush()?;
            queue.roundtrip(&mut State)?;
            eprintln!("subsurface created below GTK's surface (buffer scale {scale})");

            let display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                NonNull::new(wl_display as *mut c_void).ok_or("null wl_display")?,
            ));
            let window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(
                NonNull::new(child.id().as_ptr() as *mut c_void).ok_or("null wl_surface")?,
            ));
            let handles = Handles(display_handle, window_handle);
            std::thread::Builder::new()
                .name("probe-render".into())
                .spawn(move || {
                    let handles = handles;
                    render(handles.0, handles.1, hole, scale as u32)
                })?;

            let layer = Self { conn, subsurface, gtk_window, webview, _child: child, _queue: queue };
            Ok(layer)
        }

        /// Moves the layer under the page's hole (CSS pixels from the web view).
        pub fn place(&self, hole: Hole) {
            let (wx, wy) = self
                .webview
                .translate_coordinates(&self.gtk_window, 0, 0)
                .unwrap_or((0, 0));
            self.subsurface
                .set_position(wx + hole.x.round() as i32, wy + hole.y.round() as i32);
            // Subsurface position is parent state: GTK's next commit applies it.
            self.gtk_window.queue_draw();
            let _ = self.conn.flush();
            eprintln!("layer at ({}, {}) + hole {:?}", wx, wy, hole);
        }
    }

    /// The raw handles name objects that outlive the render thread (the
    /// window keeps them); libwayland proxies may be used from any thread.
    struct Handles(RawDisplayHandle, RawWindowHandle);
    unsafe impl Send for Handles {}

    fn find_webview(widget: &gtk::Widget) -> Option<gtk::Widget> {
        if widget.is::<webkit2gtk::WebView>() {
            return Some(widget.clone());
        }
        let container = widget.downcast_ref::<gtk::Container>()?;
        container.children().iter().find_map(find_webview)
    }

    fn render(
        display: RawDisplayHandle,
        window: RawWindowHandle,
        hole: Arc<Mutex<Option<Hole>>>,
        scale: u32,
    ) {
        let strip = std::env::var("PROBE_UPLOAD").is_ok_and(|v| v == "strip");
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            ..Default::default()
        });
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: display,
                raw_window_handle: window,
            })
        }
        .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            power_preference: wgpu::PowerPreference::LowPower,
            ..Default::default()
        }))
        .expect("adapter");
        eprintln!("adapter: {:?}", adapter.get_info());
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None)).expect("device");
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .unwrap_or(caps.formats[0]);
        let copy = caps.usages.contains(wgpu::TextureUsages::COPY_DST)
            && !std::env::var("PROBE_UPLOAD").is_ok_and(|v| v == "clear");
        eprintln!("surface format {format:?}, copy_dst {copy}, upload {}", if strip { "strip" } else { "full" });

        let mut size = (0, 0);
        let mut pattern = Vec::new();
        let (mut frames, mut busy, mut shift) = (0u32, Duration::ZERO, 0u32);
        let mut tick = Instant::now();
        loop {
            let Some(h) = *hole.lock().unwrap() else {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            };
            let want = (
                ((h.width * scale as f64).round() as u32).max(1),
                ((h.height * scale as f64).round() as u32).max(1),
            );
            if want != size {
                size = want;
                surface.configure(
                    &device,
                    &wgpu::SurfaceConfiguration {
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | if copy { wgpu::TextureUsages::COPY_DST } else { wgpu::TextureUsages::empty() },
                        format,
                        width: size.0,
                        height: size.1,
                        present_mode: wgpu::PresentMode::Fifo,
                        desired_maximum_frame_latency: 2,
                        alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                        view_formats: vec![],
                    },
                );
                pattern = make_pattern(size, format == wgpu::TextureFormat::Bgra8Unorm);
                eprintln!("surface {}x{}", size.0, size.1);
            }
            let Ok(frame) = surface.get_current_texture() else { continue };
            let started = Instant::now();
            shift = (shift + 8) % size.1;
            if copy {
                let rows = if strip { 64.min(size.1) } else { size.1 };
                // Rotate the pattern by `shift` rows: two copies, like dirty rects.
                let first = (size.1 - shift).min(rows);
                upload(&queue, &frame.texture, &pattern, size.0, shift, 0, first);
                if rows > first {
                    upload(&queue, &frame.texture, &pattern, size.0, 0, first, rows - first);
                }
                // write_texture is staged until the next submit.
                queue.submit([]);
            } else {
                let mut encoder = device.create_command_encoder(&Default::default());
                let view = frame.texture.create_view(&Default::default());
                let t = f64::from(shift) / f64::from(size.1);
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color { r: t, g: 0.3, b: 1.0 - t, a: 1.0 }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                queue.submit([encoder.finish()]);
            }
            frame.present();
            busy += started.elapsed();
            frames += 1;
            if tick.elapsed() >= Duration::from_secs(1) {
                println!(
                    "subsurface: {frames} fps, {:.2} ms of CPU per frame on the render thread",
                    busy.as_secs_f64() * 1000.0 / f64::from(frames.max(1))
                );
                (frames, busy, tick) = (0, Duration::ZERO, Instant::now());
            }
        }
    }

    fn upload(queue: &wgpu::Queue, texture: &wgpu::Texture, pattern: &[u8], width: u32, from_row: u32, to_row: u32, rows: u32) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: to_row, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &pattern[(from_row * width * 4) as usize..((from_row + rows) * width * 4) as usize],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(rows) },
            wgpu::Extent3d { width, height: rows, depth_or_array_layers: 1 },
        );
    }

    fn make_pattern((w, h): (u32, u32), bgra: bool) -> Vec<u8> {
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let (r, g, b) = ((x * 255 / w) as u8, (y * 255 / h) as u8, if (y >> 5) & 1 == 1 { 200 } else { 0 });
                data.extend_from_slice(&if bgra { [b, g, r, 255] } else { [r, g, b, 255] });
            }
        }
        data
    }
}
