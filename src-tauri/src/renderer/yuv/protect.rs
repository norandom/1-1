//! Protection tests for `yuv.rs`, written from planted faults.

use super::*;

/// An NV12 frame: every luma value, and U and V interleaved in one plane
/// with values that differ from each other.
fn nv12(width: u32, height: u32, matrix: YuvMatrix) -> YuvFrame {
    let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
    let y = (0..width * height).map(|i| (i * 7 % 256) as u8).collect();
    let uv = (0..cw * ch)
        .flat_map(|i| [(i * 13 % 256) as u8, (255 - i * 29 % 256) as u8])
        .collect();
    YuvFrame {
        width,
        height,
        y,
        y_stride: width,
        chroma: Chroma::Interleaved(uv),
        chroma_stride: cw * 2,
        matrix,
    }
}

/// A device and its queue on a real GPU, or none on a machine without one.
fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN
            | wgpu::Backends::GL
            | wgpu::Backends::METAL
            | wgpu::Backends::DX12,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    pollster::block_on(adapter.request_device(&Default::default(), None)).ok()
}

/// What `converter` draws for `frame`, read back as tight RGBA.
fn converted(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    converter: &YuvConverter,
    frame: &YuvFrame,
) -> Vec<u8> {
    let size = wgpu::Extent3d {
        width: frame.width,
        height: frame.height,
        depth_or_array_layers: 1,
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    converter.convert(device, queue, frame, &target);
    let row = frame.width * 4;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * frame.height),
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
                bytes_per_row: Some(row),
                rows_per_image: Some(frame.height),
            },
        },
        size,
    );
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::Maintain::Wait);
    let pixels = readback.slice(..).get_mapped_range().to_vec();
    pixels
}

/// The shader is told when U and V are interleaved (NV12); without that it
/// reads V from the channel that holds U. Machines without a GPU skip it with
/// a note instead of failing, as the GPU test of the file does.
#[test]
fn an_nv12_frame_converts_on_the_gpu_as_the_reference_does() {
    let Some((device, queue)) = gpu() else {
        eprintln!("skipped: no GPU adapter or device");
        return;
    };
    let converter = YuvConverter::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    for matrix in [YuvMatrix::Bt709FullRdpegfx, YuvMatrix::Bt601Limited] {
        // 64 pixels wide: rows are 256 bytes, as buffer copies require.
        let frame = nv12(64, 9, matrix);
        let gpu = converted(&device, &queue, &converter, &frame);
        let reference = frame.reference_rgba();
        let worst = gpu
            .iter()
            .zip(&reference)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        // Float rounding may land on the other side of .5.
        assert!(worst <= 1, "{matrix:?}: off by {worst}");
    }
}
