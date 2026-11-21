//! Spike: draw remote frames below a transparent WebView2 on Windows.
//!
//! PROBE_MODE=layer (default): a wgpu (DX12) swap chain in a DirectComposition
//! visual on the window's non-topmost target. DWM composes that target below
//! the window's child windows, WebView2 among them, so the page's transparent
//! hole should show the layer. Tauri creates WebView2 with the ordinary
//! (windowed) controller; wry does not expose the composition controller, so
//! this is the route that needs no fork.
//! PROBE_MODE=canvas: the same pattern drawn by the page into a canvas (the
//! current 1-1 path, without the IPC).
//! PROBE_UPLOAD=full (default) uploads the whole frame every time; strip
//! uploads 64 rows per frame, like a terminal scrolling; clear only clears.
//!
//! Prints one line a second: frames presented and the time spent per frame.

#![cfg_attr(not(windows), allow(unused))]

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
    #[cfg(windows)]
    {
        let rect = Hole { x, y, width, height };
        let _ = app.run_on_main_thread(move || {
            win::LAYER.with(|layer| {
                if let Some(layer) = layer.borrow().as_ref() {
                    layer.place(rect);
                }
            });
        });
    }
}

fn main() {
    let probe_mode = std::env::var("PROBE_MODE").unwrap_or_else(|_| "layer".into());
    tauri::Builder::default()
        .manage(Probe { mode: probe_mode.clone() })
        .invoke_handler(tauri::generate_handler![mode, fps, hole])
        .setup(move |app| {
            #[cfg(windows)]
            if probe_mode == "layer" {
                let window = app.get_webview_window("main").unwrap();
                let layer = win::Layer::new(&window)?;
                win::LAYER.with(|slot| *slot.borrow_mut() = Some(layer));
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("probe failed");
}

#[cfg(windows)]
mod win {
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::DirectComposition::{
        DCompositionCreateDevice2, IDCompositionDesktopDevice, IDCompositionTarget,
        IDCompositionVisual2,
    };
    use windows::core::{IUnknown, Interface};

    use super::Hole;

    thread_local! {
        /// DirectComposition objects are not thread-safe; they stay on the
        /// main thread, which also runs the window.
        pub static LAYER: RefCell<Option<Layer>> = const { RefCell::new(None) };
    }

    /// What the render thread draws. Its lock also keeps a resize (configure,
    /// on the main thread) out of a frame in flight.
    struct Frame {
        size: (u32, u32),
        pattern: Vec<u8>,
    }

    struct Gpu {
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        format: wgpu::TextureFormat,
        copy: bool,
        frame: Mutex<Frame>,
    }

    pub struct Layer {
        device: IDCompositionDesktopDevice,
        _target: IDCompositionTarget,
        visual: IDCompositionVisual2,
        gpu: Arc<Gpu>,
        scale: f64,
    }

    impl Layer {
        pub fn new(window: &tauri::WebviewWindow) -> Result<Self, Box<dyn std::error::Error>> {
            let hwnd = HWND(window.hwnd()?.0);
            let scale = window.scale_factor()?;
            // SAFETY: plain DirectComposition calls on the window's thread;
            // `hwnd` is the live main window.
            let (device, target, visual) = unsafe {
                let device: IDCompositionDesktopDevice =
                    DCompositionCreateDevice2(None::<&IUnknown>)?;
                // topmost = false: composed below the window's child windows.
                let target = device.CreateTargetForHwnd(hwnd, false)?;
                let visual = device.CreateVisual()?;
                target.SetRoot(&visual)?;
                (device, target, visual)
            };

            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::DX12,
                ..Default::default()
            });
            // SAFETY: the visual lives as long as `Layer`, which holds it;
            // wgpu takes its own reference.
            let surface = unsafe {
                instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CompositionVisual(
                    visual.as_raw(),
                ))
            }?;
            let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                power_preference: wgpu::PowerPreference::LowPower,
                ..Default::default()
            }))
            .ok_or("no DX12 adapter")?;
            eprintln!("adapter: {:?}", adapter.get_info());
            let (gpu_device, queue) =
                pollster::block_on(adapter.request_device(&Default::default(), None))?;
            let caps = surface.get_capabilities(&adapter);
            let format = caps
                .formats
                .iter()
                .copied()
                .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
                .unwrap_or(caps.formats[0]);
            let copy = caps.usages.contains(wgpu::TextureUsages::COPY_DST)
                && !std::env::var("PROBE_UPLOAD").is_ok_and(|v| v == "clear");
            eprintln!(
                "surface format {format:?}, alpha {:?}, copy_dst {copy}, scale {scale}",
                caps.alpha_modes
            );

            let gpu = Arc::new(Gpu {
                device: gpu_device,
                queue,
                surface,
                format,
                copy,
                frame: Mutex::new(Frame { size: (0, 0), pattern: Vec::new() }),
            });
            let render_gpu = Arc::clone(&gpu);
            std::thread::Builder::new()
                .name("probe-render".into())
                .spawn(move || render(&render_gpu))?;
            // SAFETY: as above.
            unsafe { device.Commit()? };
            eprintln!("DirectComposition visual created below the window's child windows");
            Ok(Self { device, _target: target, visual, gpu, scale })
        }

        /// Sizes the layer to the page's hole and moves it there. The hole is
        /// in CSS pixels of the web view, which fills the client area.
        pub fn place(&self, hole: Hole) {
            let size = (
                ((hole.width * self.scale).round() as u32).max(1),
                ((hole.height * self.scale).round() as u32).max(1),
            );
            {
                let mut frame = self.gpu.frame.lock().unwrap();
                if frame.size != size {
                    self.gpu.surface.configure(&self.gpu.device, &config(self.gpu.format, self.gpu.copy, size));
                    frame.pattern = make_pattern(size, self.gpu.format == wgpu::TextureFormat::Bgra8Unorm);
                    frame.size = size;
                }
            }
            let (x, y) = ((hole.x * self.scale).round(), (hole.y * self.scale).round());
            // SAFETY: main thread, objects owned by `self`.
            unsafe {
                let _ = self.visual.SetOffsetX2(x as f32);
                let _ = self.visual.SetOffsetY2(y as f32);
                // configure's SetContent and the offset apply together.
                if let Err(e) = self.device.Commit() {
                    eprintln!("Commit failed: {e}");
                }
            }
            eprintln!("layer at ({x}, {y}), {}x{} physical pixels", size.0, size.1);
        }
    }

    fn config(format: wgpu::TextureFormat, copy: bool, (width, height): (u32, u32)) -> wgpu::SurfaceConfiguration {
        wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | if copy { wgpu::TextureUsages::COPY_DST } else { wgpu::TextureUsages::empty() },
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
        }
    }

    fn render(gpu: &Gpu) {
        let strip = std::env::var("PROBE_UPLOAD").is_ok_and(|v| v == "strip");
        let (mut frames, mut busy, mut shift) = (0u32, Duration::ZERO, 0u32);
        let mut tick = Instant::now();
        loop {
            let frame = gpu.frame.lock().unwrap();
            let size = frame.size;
            if size == (0, 0) {
                drop(frame);
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            let Ok(texture) = gpu.surface.get_current_texture() else { continue };
            let started = Instant::now();
            shift = (shift + 8) % size.1;
            if gpu.copy {
                let rows = if strip { 64.min(size.1) } else { size.1 };
                // Rotate the pattern by `shift` rows: two copies, like dirty rects.
                let first = (size.1 - shift).min(rows);
                upload(&gpu.queue, &texture.texture, &frame.pattern, size.0, shift, 0, first);
                if rows > first {
                    upload(&gpu.queue, &texture.texture, &frame.pattern, size.0, 0, first, rows - first);
                }
                // write_texture is staged until the next submit.
                gpu.queue.submit([]);
            } else {
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                let view = texture.texture.create_view(&Default::default());
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
                gpu.queue.submit([encoder.finish()]);
            }
            texture.present();
            busy += started.elapsed();
            drop(frame);
            frames += 1;
            if tick.elapsed() >= Duration::from_secs(1) {
                println!(
                    "layer: {frames} fps, {:.2} ms of CPU per frame on the render thread",
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
