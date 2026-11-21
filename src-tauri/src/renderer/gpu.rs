//! The native layer's render thread (spec rdp-native-renderer, task 4.1).
//!
//! One thread per window. It waits on the layer's `Wake` signal, which the
//! layer rings on geometry changes, the view on changes of the visible
//! session, and every registered frame store on new damage. It keeps one
//! texture with the visible session's picture, uploads only the damaged
//! rectangle, clears the layer to the page's background, draws the picture
//! into the view's destination rectangle and presents only when something
//! changed. Failures it cannot recover from are reported once through
//! `on_failure`; the app then falls back to the canvas (task 6.1).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;

use super::frame_stores::FrameStores;
use super::native_layer::{Geometry, LayerTarget, SurfaceSource, Wake};
use super::shared_frame::Damage;
use super::yuv::{FrameSource, YuvConverter};

/// A rectangle in floating point (CSS pixels or remote pixels).
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// What the page shows where (task 4.2 sets it from `SessionCanvas`).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    /// The session on screen; `None` draws only the background.
    pub session: Option<String>,
    /// Where the picture goes, in CSS pixels of the web view.
    pub dest: Rect,
    /// Which part of the remote desktop, in remote pixels.
    pub source: Rect,
    /// Linear filtering (fit mode); nearest otherwise.
    pub smooth: bool,
    /// The page's background behind the session, 0–255 RGB.
    pub background: [u8; 3],
}

/// The view, shared between the page's commands and the render thread.
pub struct RenderShared {
    view: Mutex<View>,
    stores: Arc<FrameStores>,
    /// Frames presented per session, for the Performance HUD's FPS.
    presented: Mutex<std::collections::HashMap<String, u64>>,
}

impl RenderShared {
    pub fn new(stores: Arc<FrameStores>) -> Arc<Self> {
        Arc::new(Self {
            view: Mutex::new(View::default()),
            stores,
            presented: Mutex::default(),
        })
    }

    /// Replaces the view and wakes the render thread when it changed.
    pub fn set_view(&self, view: View) {
        let mut current = self.view.lock().unwrap();
        if *current != view {
            *current = view;
            drop(current);
            self.stores.wake().notify();
        }
    }

    fn view(&self) -> View {
        self.view.lock().unwrap().clone()
    }

    fn count_present(&self, session: &str) {
        *self
            .presented
            .lock()
            .unwrap()
            .entry(session.to_owned())
            .or_default() += 1;
    }

    /// Frames presented for a session so far.
    pub fn presented(&self, session: &str) -> u64 {
        self.presented
            .lock()
            .unwrap()
            .get(session)
            .copied()
            .unwrap_or(0)
    }
}

/// The page reports what it shows over the native layer (task 4.2): the
/// visible session and where, or `null` when no session is on screen.
#[tauri::command]
pub fn native_view(view: Option<View>, shared: tauri::State<'_, Arc<RenderShared>>) {
    shared.set_view(view.unwrap_or_default());
}

/// Frames the native layer presented for a session; the HUD's FPS on the
/// native path, where no frames reach the canvas.
#[tauri::command]
pub fn native_frames(session_id: String, shared: tauri::State<'_, Arc<RenderShared>>) -> u64 {
    shared.presented(&session_id)
}

/// Uniforms for `native_layer.wgsl`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    /// Normalized device coordinates: left, top, right, bottom.
    pub dst: [f32; 4],
    /// Texture coordinates: left, top, right, bottom.
    pub src: [f32; 4],
    pub smooth: bool,
}

impl Params {
    /// Maps the view onto the layer (`logical` CSS size) and the texture.
    pub fn new(view: &View, logical: (u32, u32), texture: (u32, u32)) -> Option<Self> {
        let (lw, lh) = (f64::from(logical.0), f64::from(logical.1));
        let (tw, th) = (f64::from(texture.0), f64::from(texture.1));
        let (d, s) = (view.dest, view.source);
        if lw <= 0.0 || lh <= 0.0 || tw <= 0.0 || th <= 0.0 || d.width <= 0.0 || d.height <= 0.0 {
            return None;
        }
        let ndc_x = |x: f64| (x / lw * 2.0 - 1.0) as f32;
        let ndc_y = |y: f64| (1.0 - y / lh * 2.0) as f32;
        Some(Self {
            dst: [
                ndc_x(d.x),
                ndc_y(d.y),
                ndc_x(d.x + d.width),
                ndc_y(d.y + d.height),
            ],
            src: [
                (s.x / tw) as f32,
                (s.y / th) as f32,
                ((s.x + s.width) / tw) as f32,
                ((s.y + s.height) / th) as f32,
            ],
            smooth: view.smooth,
        })
    }

    fn bytes(&self) -> [u8; 48] {
        let mut out = [0u8; 48];
        for (i, value) in self.dst.iter().chain(self.src.iter()).enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        out[32..36].copy_from_slice(&u32::from(self.smooth).to_le_bytes());
        out
    }
}

/// Starts the render thread for the layer.
pub fn spawn(
    target: LayerTarget,
    shared: Arc<RenderShared>,
    on_failure: Box<dyn Fn(String) + Send>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("native-render".into())
        .spawn(move || {
            let result =
                Renderer::new(&target).and_then(|mut renderer| renderer.run(&target, &shared));
            if let Err(error) = result {
                log::error!("Native layer failed: {error}");
                on_failure(error);
            }
        })
        .map(drop)
}

struct Picture {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    size: (u32, u32),
}

struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    nearest: wgpu::Sampler,
    linear: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    picture: Option<Picture>,
    /// Created on the first YUV frame (H.264, not yet produced).
    #[allow(dead_code, reason = "read once H.264 ships")]
    yuv: Option<YuvConverter>,
    configured: Geometry,
    shown: Option<String>,
    /// Windows: `configure` reaches the DirectComposition visual, so it runs
    /// under the composition's lock and is committed (amendment 5a).
    #[cfg(windows)]
    composition: Option<Arc<super::native_layer::dcomp::Composition>>,
}

/// Consecutive failed frames before the layer gives up (design, errors).
const FAILURES_BEFORE_FALLBACK: u32 = 3;

impl Renderer {
    fn new(target: &LayerTarget) -> Result<Self, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: super::display_path::layer_backends(),
            ..Default::default()
        });
        // SAFETY: the layer keeps the Wayland objects and the visual alive for
        // the window's lifetime; wgpu takes its own reference to the visual.
        let surface = unsafe {
            match &target.surface {
                SurfaceSource::Raw(display, window) => {
                    instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                        raw_display_handle: *display,
                        raw_window_handle: *window,
                    })
                }
                #[cfg(windows)]
                SurfaceSource::Composition(composition) => instance.create_surface_unsafe(
                    wgpu::SurfaceTargetUnsafe::CompositionVisual(composition.visual()),
                ),
            }
        }
        .map_err(|e| format!("surface: {e}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .ok_or("no GPU adapter for the layer")?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("native-layer"),
                ..Default::default()
            },
            None,
        ))
        .map_err(|e| format!("device: {e}"))?;
        device.on_uncaptured_error(Box::new(|error| {
            log::error!("Native layer GPU error: {error}")
        }));
        let caps = surface.get_capabilities(&adapter);
        // Non-sRGB, as the canvas: remote bytes are shown as they are.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
                )
            })
            .or_else(|| caps.formats.first().copied())
            .ok_or("the surface offers no format")?;
        log::info!(
            "Native layer renders with {} ({:?}, {format:?})",
            adapter.get_info().name,
            adapter.get_info().backend
        );

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("native-layer"),
            source: wgpu::ShaderSource::Wgsl(include_str!("native_layer.wgsl").into()),
        });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty,
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("native-layer"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    1,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    2,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    3,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("native-layer"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("native-layer"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let sampler = |filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        let nearest = sampler(wgpu::FilterMode::Nearest);
        let linear = sampler(wgpu::FilterMode::Linear);
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("native-layer"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            device,
            queue,
            surface,
            format,
            pipeline,
            layout,
            nearest,
            linear,
            uniforms,
            picture: None,
            yuv: None,
            configured: Geometry::default(),
            shown: None,
            #[cfg(windows)]
            composition: match &target.surface {
                SurfaceSource::Composition(composition) => Some(Arc::clone(composition)),
                SurfaceSource::Raw(..) => None,
            },
        })
    }

    fn run(&mut self, target: &LayerTarget, shared: &RenderShared) -> Result<(), String> {
        let wake: &Wake = &target.wake;
        // For checking the fallback by hand: fail after this many frames.
        let mut fail_after: Option<u64> = std::env::var("ONE_ONE_RENDERER_FAIL_AFTER")
            .ok()
            .and_then(|value| value.parse().ok());
        let mut seen = 0;
        let mut failures = 0;
        let mut drawn_view: Option<View> = None;
        loop {
            seen = wake.wait(seen, Duration::from_secs(1));
            let geometry = *target.geometry.lock().unwrap();
            if geometry.buffer_width == 0 {
                continue;
            }
            let view = shared.view();
            let mut changed = geometry != self.configured || drawn_view.as_ref() != Some(&view);
            if geometry != self.configured {
                self.configure(geometry);
            }

            // Bring the texture up to date with the visible session.
            let store = view.session.as_deref().and_then(|id| shared.stores.get(id));
            if view.session != self.shown {
                self.shown = view.session.clone();
                if let Some(store) = &store {
                    self.upload(&store.take_full());
                    // Pending damage is part of the full picture now.
                    let _ = store.take_damage();
                }
                changed = true;
            } else if let Some(store) = &store
                && let Some(damage) = store.take_damage()
            {
                self.upload(&damage);
                changed = true;
            }
            if !changed {
                continue;
            }

            match self.draw(&view, geometry, store.is_some()) {
                Ok(()) => {
                    failures = 0;
                    if let Some(left) = fail_after.as_mut() {
                        if *left == 0 {
                            return Err("injected by ONE_ONE_RENDERER_FAIL_AFTER".into());
                        }
                        *left -= 1;
                    }
                    if let (Some(session), Some(_)) = (view.session.as_deref(), &store) {
                        shared.count_present(session);
                    }
                    drawn_view = Some(view);
                }
                Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                    self.configure(geometry);
                    drawn_view = None;
                    wake.notify();
                }
                Err(wgpu::SurfaceError::Timeout) => {
                    // The window is not shown; try again on the next change.
                    drawn_view = None;
                }
                Err(error) => {
                    failures += 1;
                    drawn_view = None;
                    log::warn!("Native layer frame failed ({failures}): {error}");
                    if failures >= FAILURES_BEFORE_FALLBACK
                        || matches!(error, wgpu::SurfaceError::OutOfMemory)
                    {
                        return Err(error.to_string());
                    }
                }
            }
        }
    }

    fn configure(&mut self, geometry: Geometry) {
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: self.format,
            width: geometry.buffer_width,
            height: geometry.buffer_height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
        };
        let configure = || self.surface.configure(&self.device, &config);
        #[cfg(windows)]
        match &self.composition {
            Some(composition) => composition.apply(configure),
            None => configure(),
        }
        #[cfg(not(windows))]
        configure();
        self.configured = geometry;
    }

    /// One entry for both inputs (requirement 7.1): RGBA damage, or a whole
    /// YUV picture converted on the GPU into the same texture.
    #[allow(
        dead_code,
        reason = "the RGBA path calls upload directly until H.264 ships"
    )]
    fn upload_source(&mut self, source: FrameSource) {
        match source {
            FrameSource::Rgba(damage) => self.upload(&damage),
            FrameSource::Yuv420(frame) => {
                let size = (frame.width, frame.height);
                if size.0 == 0 || size.1 == 0 {
                    return;
                }
                if self.picture.as_ref().is_none_or(|p| p.size != size) {
                    self.picture = Some(self.create_picture(size));
                }
                let converter = self.yuv.get_or_insert_with(|| {
                    YuvConverter::new(&self.device, wgpu::TextureFormat::Rgba8Unorm)
                });
                let picture = self.picture.as_ref().expect("created above");
                converter.convert(&self.device, &self.queue, &frame, &picture.texture);
            }
        }
    }

    /// Writes damaged pixels into the texture, recreating it for a full
    /// frame at a new size.
    fn upload(&mut self, damage: &Damage) {
        let size = (damage.frame_width, damage.frame_height);
        if size.0 == 0 || size.1 == 0 || damage.width == 0 || damage.height == 0 {
            return;
        }
        if self.picture.as_ref().is_none_or(|p| p.size != size) {
            if !damage.full {
                return; // A partial update for a texture that no longer matches.
            }
            self.picture = Some(self.create_picture(size));
        }
        let picture = self.picture.as_ref().expect("created above");
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &picture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: damage.x,
                    y: damage.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &damage.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(damage.width * 4),
                rows_per_image: Some(damage.height),
            },
            wgpu::Extent3d {
                width: damage.width,
                height: damage.height,
                depth_or_array_layers: 1,
            },
        );
    }

    fn create_picture(&self, size: (u32, u32)) -> Picture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("session-picture"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            // RENDER_ATTACHMENT: the YUV converter renders into it.
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("session-picture"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.nearest),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.linear),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.uniforms.as_entire_binding(),
                },
            ],
        });
        Picture {
            texture,
            bind_group,
            size,
        }
    }

    fn draw(
        &mut self,
        view: &View,
        geometry: Geometry,
        has_picture: bool,
    ) -> Result<(), wgpu::SurfaceError> {
        let frame = self.surface.get_current_texture()?;
        let target = frame.texture.create_view(&Default::default());
        let [r, g, b] = view.background.map(|c| f64::from(c) / 255.0);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("native-layer"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            let picture = self.picture.as_ref().filter(|_| has_picture);
            let params = picture.and_then(|p| {
                Params::new(
                    view,
                    (geometry.logical_width, geometry.logical_height),
                    p.size,
                )
            });
            if let (Some(picture), Some(params)) = (picture, params) {
                self.queue.write_buffer(&self.uniforms, 0, &params.bytes());
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &picture.bind_group, &[]);
                pass.draw(0..4, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        frame.present();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(dest: Rect, source: Rect) -> View {
        View {
            session: Some("s".into()),
            dest,
            source,
            smooth: false,
            background: [0, 0, 0],
        }
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn a_picture_filling_the_layer_spans_device_coordinates() {
        let p = Params::new(
            &view(rect(0.0, 0.0, 800.0, 600.0), rect(0.0, 0.0, 1600.0, 1200.0)),
            (800, 600),
            (1600, 1200),
        )
        .unwrap();
        assert_eq!(p.dst, [-1.0, 1.0, 1.0, -1.0]);
        assert_eq!(p.src, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn a_centered_fit_and_a_scrolled_native_view() {
        // Fit: 400x300 picture centred in 800x600, below a 50 px header.
        let fit = Params::new(
            &view(
                rect(200.0, 150.0, 400.0, 300.0),
                rect(0.0, 0.0, 1920.0, 1080.0),
            ),
            (800, 600),
            (1920, 1080),
        )
        .unwrap();
        assert_eq!(fit.dst, [-0.5, 0.5, 0.5, -0.5]);
        // Native, scrolled: the visible quarter of the desktop.
        let native = Params::new(
            &view(
                rect(0.0, 0.0, 800.0, 600.0),
                rect(960.0, 540.0, 960.0, 540.0),
            ),
            (800, 600),
            (1920, 1080),
        )
        .unwrap();
        assert_eq!(native.src, [0.5, 0.5, 1.0, 1.0]);
    }

    #[test]
    fn empty_or_degenerate_views_draw_nothing() {
        let v = view(rect(0.0, 0.0, 0.0, 10.0), rect(0.0, 0.0, 10.0, 10.0));
        assert!(Params::new(&v, (10, 10), (10, 10)).is_none());
        let v = view(rect(0.0, 0.0, 10.0, 10.0), rect(0.0, 0.0, 10.0, 10.0));
        assert!(Params::new(&v, (0, 10), (10, 10)).is_none());
        assert!(Params::new(&v, (10, 10), (0, 0)).is_none());
    }

    #[test]
    fn uniform_layout_matches_the_shader() {
        let p = Params {
            dst: [1.0, 2.0, 3.0, 4.0],
            src: [5.0, 6.0, 7.0, 8.0],
            smooth: true,
        };
        let bytes = p.bytes();
        assert_eq!(f32::from_le_bytes(bytes[0..4].try_into().unwrap()), 1.0);
        assert_eq!(f32::from_le_bytes(bytes[28..32].try_into().unwrap()), 8.0);
        assert_eq!(u32::from_le_bytes(bytes[32..36].try_into().unwrap()), 1);
    }

    #[test]
    fn setting_the_same_view_does_not_wake_the_render_thread() {
        let stores = Arc::new(FrameStores::default());
        let shared = RenderShared::new(Arc::clone(&stores));
        let start = stores.wake().wait(u64::MAX, Duration::ZERO);
        let v = view(rect(0.0, 0.0, 1.0, 1.0), rect(0.0, 0.0, 1.0, 1.0));
        shared.set_view(v.clone());
        let after_first = stores.wake().wait(u64::MAX, Duration::ZERO);
        assert_eq!(after_first, start + 1);
        shared.set_view(v);
        assert_eq!(stores.wake().wait(u64::MAX, Duration::ZERO), after_first);
    }
}
