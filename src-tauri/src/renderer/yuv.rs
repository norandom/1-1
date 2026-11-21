//! YUV 4:2:0 input for the native layer (spec rdp-native-renderer, task 8.1).
//!
//! H.264 is parked (branch `spike/h264-hw-decode`). When it comes, decoded
//! pictures arrive as YUV planes; converting them on the GPU keeps a hardware
//! decoder's advantage, where the canvas path would convert on the CPU and
//! repaint the window. Nothing produces `FrameSource::Yuv420` yet, so sessions
//! behave exactly as before (requirement 7.3).

// Nothing produces YUV frames until H.264 ships; the module stays tested.
#![allow(
    dead_code,
    reason = "used once H.264 ships (spec rdp-native-renderer 7.1)"
)]

use super::shared_frame::Damage;

/// What the render thread uploads into a session's picture texture.
pub enum FrameSource {
    /// Damaged RGBA pixels (ClearCodec and everything else today).
    Rgba(Damage),
    /// A whole decoded picture.
    Yuv420(YuvFrame),
}

/// How YUV maps to RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YuvMatrix {
    /// Full-range BT.709 in MS-RDPEGFX's fixed-point form (AVC420 streams,
    /// [MS-RDPEGFX] 3.3.8.3.1), rounding down like `ironrdp-egfx`.
    Bt709FullRdpegfx,
    /// Limited-range BT.601 (MS-RDPEVOR video), rounding to nearest.
    Bt601Limited,
}

/// The chroma planes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chroma {
    /// I420: separate U and V planes.
    Planar { u: Vec<u8>, v: Vec<u8> },
    /// NV12: U and V interleaved in one plane.
    Interleaved(Vec<u8>),
}

/// A decoded 4:2:0 picture; chroma is half the luma size, rounded up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YuvFrame {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub y_stride: u32,
    pub chroma: Chroma,
    /// Bytes per chroma row (both U and V together for NV12).
    pub chroma_stride: u32,
    pub matrix: YuvMatrix,
}

/// The shader's parameters for a matrix: offsets, scales, rounding and the
/// rows of the 3x3 matrix. Shared by the GPU and the CPU reference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coefficients {
    pub luma_offset: f32,
    pub chroma_offset: f32,
    pub rounding: f32,
    pub luma_scale: f32,
    pub chroma_scale: f32,
    pub rows: [[f32; 3]; 3],
}

impl YuvMatrix {
    pub fn coefficients(self) -> Coefficients {
        match self {
            // R = (256Y + 403V) >> 8, G = (256Y - 48U - 120V) >> 8,
            // B = (256Y + 475U) >> 8; all exact in binary floating point.
            Self::Bt709FullRdpegfx => Coefficients {
                luma_offset: 0.0,
                chroma_offset: 128.0,
                rounding: 0.0,
                luma_scale: 1.0,
                chroma_scale: 1.0,
                rows: [
                    [1.0, 0.0, 403.0 / 256.0],
                    [1.0, -48.0 / 256.0, -120.0 / 256.0],
                    [1.0, 475.0 / 256.0, 0.0],
                ],
            },
            Self::Bt601Limited => Coefficients {
                luma_offset: 16.0,
                chroma_offset: 128.0,
                rounding: 0.5,
                luma_scale: 255.0 / 219.0,
                chroma_scale: 255.0 / 224.0,
                rows: [
                    [1.0, 0.0, 1.402],
                    [1.0, -0.344_136, -0.714_136],
                    [1.0, 1.772, 0.0],
                ],
            },
        }
    }

    /// The uniform buffer for `yuv_convert.wgsl`.
    pub fn uniform_bytes(self, interleaved: bool) -> [u8; 96] {
        let c = self.coefficients();
        let mut floats = [0f32; 20];
        floats[0..4].copy_from_slice(&[c.luma_offset, c.chroma_offset, c.rounding, 0.0]);
        floats[4..8].copy_from_slice(&[c.luma_scale, c.chroma_scale, 0.0, 0.0]);
        for (row, values) in c.rows.iter().enumerate() {
            floats[8 + row * 4..11 + row * 4].copy_from_slice(values);
        }
        let mut out = [0u8; 96];
        for (i, value) in floats.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        out[80..84].copy_from_slice(&u32::from(interleaved).to_le_bytes());
        out
    }
}

impl YuvFrame {
    pub fn chroma_size(&self) -> (u32, u32) {
        (self.width.div_ceil(2), self.height.div_ceil(2))
    }

    fn sample(&self, x: u32, y: u32) -> (u8, u8, u8) {
        let luma = self.y[(y * self.y_stride + x) as usize];
        let (cx, cy) = (x / 2, y / 2);
        let (u, v) = match &self.chroma {
            Chroma::Planar { u, v } => {
                let at = (cy * self.chroma_stride + cx) as usize;
                (u[at], v[at])
            }
            Chroma::Interleaved(uv) => {
                let at = (cy * self.chroma_stride + cx * 2) as usize;
                (uv[at], uv[at + 1])
            }
        };
        (luma, u, v)
    }

    /// The conversion the shader performs, on the CPU, as tight RGBA.
    pub fn reference_rgba(&self) -> Vec<u8> {
        let c = self.matrix.coefficients();
        let mut out = Vec::with_capacity((self.width * self.height * 4) as usize);
        for y in 0..self.height {
            for x in 0..self.width {
                let (luma, u, v) = self.sample(x, y);
                let yuv = [
                    (f32::from(luma) - c.luma_offset) * c.luma_scale,
                    (f32::from(u) - c.chroma_offset) * c.chroma_scale,
                    (f32::from(v) - c.chroma_offset) * c.chroma_scale,
                ];
                for row in c.rows {
                    let value = row[0] * yuv[0] + row[1] * yuv[1] + row[2] * yuv[2];
                    out.push((value + c.rounding).floor().clamp(0.0, 255.0) as u8);
                }
                out.push(255);
            }
        }
        out
    }
}

/// Converts YUV planes into an RGBA texture on the GPU.
pub struct YuvConverter {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
}

impl YuvConverter {
    /// `target` is the format of the picture texture (RGBA8).
    pub fn new(device: &wgpu::Device, target: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yuv-convert"),
            source: wgpu::ShaderSource::Wgsl(include_str!("yuv_convert.wgsl").into()),
        });
        let plane = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yuv-convert"),
            entries: &[
                plane(0),
                plane(1),
                plane(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yuv-convert"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("yuv-convert"),
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
                    format: target,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yuv-convert"),
            size: 96,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            uniforms,
        }
    }

    /// Converts `frame` into `target`, which must be `frame`'s size and
    /// allow rendering.
    pub fn convert(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &YuvFrame,
        target: &wgpu::Texture,
    ) {
        let plane = |label, format, (width, height): (u32, u32), data: &[u8], stride: u32| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            texture.create_view(&Default::default())
        };
        let size = (frame.width, frame.height);
        let chroma_size = frame.chroma_size();
        let luma = plane(
            "yuv-y",
            wgpu::TextureFormat::R8Unorm,
            size,
            &frame.y,
            frame.y_stride,
        );
        let (u, v, interleaved) = match &frame.chroma {
            Chroma::Planar { u, v } => (
                plane(
                    "yuv-u",
                    wgpu::TextureFormat::R8Unorm,
                    chroma_size,
                    u,
                    frame.chroma_stride,
                ),
                plane(
                    "yuv-v",
                    wgpu::TextureFormat::R8Unorm,
                    chroma_size,
                    v,
                    frame.chroma_stride,
                ),
                false,
            ),
            Chroma::Interleaved(uv) => {
                let view = plane(
                    "yuv-uv",
                    wgpu::TextureFormat::Rg8Unorm,
                    chroma_size,
                    uv,
                    frame.chroma_stride,
                );
                (view.clone(), view, true)
            }
        };
        queue.write_buffer(&self.uniforms, 0, &frame.matrix.uniform_bytes(interleaved));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yuv-convert"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&luma),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&u),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&v),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.uniforms.as_entire_binding(),
                },
            ],
        });
        let view = target.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("yuv-convert"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture with every luma value and a spread of chroma values.
    fn synthetic(width: u32, height: u32, matrix: YuvMatrix) -> YuvFrame {
        let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
        let y = (0..width * height).map(|i| (i * 7 % 256) as u8).collect();
        let u = (0..cw * ch).map(|i| (i * 13 % 256) as u8).collect();
        let v = (0..cw * ch).map(|i| (255 - i * 29 % 256) as u8).collect();
        YuvFrame {
            width,
            height,
            y,
            y_stride: width,
            chroma: Chroma::Planar { u, v },
            chroma_stride: cw,
            matrix,
        }
    }

    #[test]
    fn the_reference_matches_ironrdp_egfx_exactly() {
        let frame = synthetic(37, 21, YuvMatrix::Bt709FullRdpegfx);
        let Chroma::Planar { u, v } = &frame.chroma else {
            unreachable!()
        };
        let upstream = ironrdp_egfx::decode::Yuv420Frame::new(
            37,
            21,
            frame.y.clone(),
            37,
            u.clone(),
            v.clone(),
            frame.chroma_stride as usize,
        )
        .unwrap()
        .to_rgba();
        assert_eq!(frame.reference_rgba(), upstream.data());
    }

    #[test]
    fn nv12_and_i420_give_the_same_picture() {
        let planar = synthetic(9, 5, YuvMatrix::Bt601Limited);
        let Chroma::Planar { u, v } = &planar.chroma else {
            unreachable!()
        };
        let uv = u.iter().zip(v).flat_map(|(u, v)| [*u, *v]).collect();
        let interleaved = YuvFrame {
            chroma: Chroma::Interleaved(uv),
            chroma_stride: planar.chroma_stride * 2,
            ..planar.clone()
        };
        assert_eq!(interleaved.reference_rgba(), planar.reference_rgba());
    }

    #[test]
    fn limited_range_maps_black_and_white() {
        let frame = |y| YuvFrame {
            width: 1,
            height: 1,
            y: vec![y],
            y_stride: 1,
            chroma: Chroma::Planar {
                u: vec![128],
                v: vec![128],
            },
            chroma_stride: 1,
            matrix: YuvMatrix::Bt601Limited,
        };
        assert_eq!(frame(16).reference_rgba(), vec![0, 0, 0, 255]);
        assert_eq!(frame(235).reference_rgba(), vec![255, 255, 255, 255]);
    }

    /// The shader against the reference, on a real GPU. Machines without one
    /// (CI) skip it with a note instead of failing.
    #[test]
    fn the_gpu_conversion_matches_the_reference() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN
                | wgpu::Backends::GL
                | wgpu::Backends::METAL
                | wgpu::Backends::DX12,
            ..Default::default()
        });
        let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default()))
        else {
            eprintln!("skipped: no GPU adapter");
            return;
        };
        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&Default::default(), None))
        else {
            eprintln!("skipped: no GPU device");
            return;
        };
        let converter = YuvConverter::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        for matrix in [YuvMatrix::Bt709FullRdpegfx, YuvMatrix::Bt601Limited] {
            // 64 pixels wide: rows are 256 bytes, as buffer copies require.
            let frame = synthetic(64, 9, matrix);
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 64,
                    height: 9,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            converter.convert(&device, &queue, &frame, &target);
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 64 * 9 * 4,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(64 * 4),
                        rows_per_image: Some(9),
                    },
                },
                target.size(),
            );
            queue.submit([encoder.finish()]);
            readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::Maintain::Wait);
            let gpu = readback.slice(..).get_mapped_range().to_vec();
            let reference = frame.reference_rgba();
            let worst = gpu
                .iter()
                .zip(&reference)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            match matrix {
                YuvMatrix::Bt709FullRdpegfx => {
                    assert_eq!(gpu, reference, "exact, like ironrdp-egfx")
                }
                // Float rounding may land on the other side of .5.
                YuvMatrix::Bt601Limited => assert!(worst <= 1, "off by {worst}"),
            }
        }
    }
}
