# Zero-copy route from VA-API into the native layer

Written 21 November 2026 for spec rdp-native-renderer, task 8.2. Nothing here
is built yet; the H.264 spec owns it.

## Why

This probe measured the cost of the path through system memory: with
`vah264dec`, CPU time per frame is close to the decode latency, mostly the copy
out of GPU memory, and upstream's `Yuv420Frame::to_rgba` (CPU, whole picture)
costs more than decoding (about 17 ms at 1080p). The native layer on master
(`src-tauri/src/renderer/`) already converts YUV on the GPU
(`yuv.rs`, `yuv_convert.wgsl`, tested exact against `ironrdp-egfx` for the
MS-RDPEGFX BT.709 full-range transform). What is left is to stop copying the
decoded picture through the CPU at all.

## Steps

1. **Export the decoded surface.** GStreamer 1.24+: let `vah264dec` negotiate
   `video/x-raw(memory:DMABuf)` with `format=DMA_DRM` into the `appsink`, read
   the fd, DRM fourcc (NV12) and modifier from the buffer
   (`gst_dmabuf_memory_get_fd`, `GstVideoInfoDmaDrm`). Without GStreamer:
   `vaExportSurfaceHandle(VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2)` gives the
   same, per plane (fd, offset, pitch, modifier). Wait for the decode first
   (`vaSyncSurface`, or GStreamer's buffer being ready).
2. **Import into Vulkan.** The device needs `VK_KHR_external_memory_fd`,
   `VK_EXT_external_memory_dma_buf` and `VK_EXT_image_drm_format_modifier`.
   Create one image per plane, Y as `R8_UNORM` and UV as `R8G8_UNORM`, each
   with `VkImageDrmFormatModifierExplicitCreateInfoEXT` (the plane's offset and
   pitch) and `VkExternalMemoryImageCreateInfo`, then bind memory imported with
   `VkImportMemoryFdInfoKHR`. Two images instead of one multi-planar
   `G8_B8R8_2PLANE_420_UNORM` image match the converter's NV12 path (an R8 luma
   and an RG8 chroma texture) without a sampler conversion.
3. **Hand the images to wgpu.** wgpu 24 has no public import API. Wrap each
   `VkImage` with `wgpu_hal::vulkan::Device::texture_from_raw` and
   `wgpu::Device::create_texture_from_hal::<wgpu_hal::api::Vulkan>` (both
   `unsafe`). The render thread's device must have been opened with the three
   extensions above: create it through `wgpu_hal::vulkan::Adapter` with the
   extra extensions (`Adapter::device_from_raw` or the open call that accepts
   them in the wgpu version then in use), then
   `wgpu::Adapter::create_device_from_hal`.
4. **Convert.** Feed the two textures to `YuvConverter` as an NV12 frame
   (it takes `R8Unorm` luma and `Rg8Unorm` chroma today; it needs a variant
   that takes texture views instead of byte planes). The picture texture and
   everything after it stay as they are.
5. **AVC444.** Two 4:2:0 pictures per frame ([MS-RDPEGFX] 3.3.8.3.2). Hardware
   decoders handle 4:2:0 only, so both views decode as above and a second
   shader recombines them into 4:4:4 before the color conversion.

## Risks

- The fd's lifetime: the image must not outlive GStreamer's buffer; keep the
  buffer until the frame is presented.
- Modifiers: Intel and AMD export tiled modifiers; the import must use the
  exported modifier, and fall back to the copy path when the driver refuses it.
- Synchronization: DMA-BUF implicit sync usually covers decode-to-sample on
  Mesa; if not, export a sync file (`DMA_BUF_IOCTL_EXPORT_SYNC_FILE`) and
  import it as a semaphore.
- A later wgpu with a public external-memory API would remove step 3's
  `wgpu-hal` code; check before building.
