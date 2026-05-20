//! Round-trip integration test for the DMABUF → Vulkan → wgpu import path.
//!
//! Allocates a Vulkan image with `VK_EXT_image_drm_format_modifier` linear
//! tiling backed by exportable device memory, clears it to a known color via
//! `vkCmdClearColorImage`, exports the underlying allocation as a DMABUF fd,
//! then hands it to grafting's `WgpuTextureImporter`. The imported wgpu
//! texture is read back to CPU and verified against the clear color.
//!
//! This is the only end-to-end coverage of `NativeFrame::VulkanExternalImage`
//! until a real producer (e.g. WPE) is integrated.
//!
//! Marked `#[ignore]` because it requires:
//!   - A Vulkan-capable wgpu adapter
//!   - `VK_EXT_image_drm_format_modifier` (Mesa exposes this; not all CI VMs do)
//!   - `VK_EXT_external_memory_dma_buf`
//!
//! Run with: `cargo test --test dmabuf_roundtrip -- --ignored --nocapture`

#![cfg(target_os = "linux")]

use ash::vk;
use dpi::PhysicalSize;
use grafting::{
    CapabilityStatus, HostWgpuContext, ImportOptions, InteropBackend, NativeFrame, SyncMechanism,
    TextureImporter, VulkanExternalImage, WgpuTextureImporter,
};
use pollster::FutureExt;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
const CLEAR_FLOAT: [f32; 4] = [1.0, 0.0, 0.5, 1.0];
const CLEAR_BYTES: [u8; 4] = [255, 0, 128, 255];
const DRM_FORMAT_MOD_LINEAR: u64 = 0;

#[test]
#[ignore = "requires VK_EXT_image_drm_format_modifier; run manually with --ignored"]
fn dmabuf_clear_roundtrip() {
    let host = setup_vulkan_host();

    let caps = host.capabilities();
    assert_eq!(caps.host_backend, InteropBackend::Vulkan);
    assert_eq!(caps.vulkan_external_image, CapabilityStatus::Supported);

    let readback_device = host.device.clone();
    let readback_queue = host.queue.clone();

    let exported = unsafe { allocate_clear_and_export(&host, WIDTH, HEIGHT, CLEAR_FLOAT) };

    let frame = VulkanExternalImage {
        size: PhysicalSize::new(WIDTH, HEIGHT),
        format: wgpu::TextureFormat::Rgba8Unorm,
        generation: 1,
        producer_sync: SyncMechanism::None,
        dmabuf_fd: exported.fd,
        dmabuf_offset: exported.offset,
        dmabuf_stride: exported.row_pitch,
        drm_modifier: exported.modifier,
        wait_semaphore_fd: None,
    };

    let importer = WgpuTextureImporter::new(host);
    let imported = importer
        .import_frame(
            &NativeFrame::VulkanExternalImage(frame),
            &ImportOptions::default(),
        )
        .expect("import_frame");

    let pixels = readback_rgba(
        &readback_device,
        &readback_queue,
        &imported.texture,
        WIDTH,
        HEIGHT,
    );

    for &(x, y) in &[(0, 0), (WIDTH / 2, HEIGHT / 2), (WIDTH - 1, HEIGHT - 1)] {
        let i = ((y * WIDTH + x) * 4) as usize;
        let actual = &pixels[i..i + 4];
        assert_eq!(actual, &CLEAR_BYTES, "pixel ({x}, {y}) mismatch");
    }

    // Producer-side resources are released here; the consumer's imported
    // wgpu texture retains its own VkImage/VkDeviceMemory bound to the
    // imported dmabuf, which the kernel keeps alive via the dup'd fd.
    drop(exported);
}

fn setup_vulkan_host() -> HostWgpuContext {
    let mut instance_desc = wgpu::InstanceDescriptor::new_without_display_handle();
    instance_desc.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(instance_desc);
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        })
        .block_on()
        .expect("no Vulkan adapter available");
    let desc = wgpu::DeviceDescriptor {
        label: Some("dmabuf-roundtrip-device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    };
    grafting::vulkan_dmabuf::create_dmabuf_host_context(&adapter, &desc)
        .expect("create_dmabuf_host_context")
}

struct ExportedDmabuf {
    fd: i32,
    offset: u64,
    row_pitch: u64,
    modifier: u64,
    _producer_state: ProducerState,
}

struct ProducerState {
    vk_device: ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
    cmd_pool: vk::CommandPool,
}

impl Drop for ProducerState {
    fn drop(&mut self) {
        unsafe {
            self.vk_device.destroy_command_pool(self.cmd_pool, None);
            self.vk_device.destroy_image(self.image, None);
            self.vk_device.free_memory(self.memory, None);
        }
    }
}

unsafe fn allocate_clear_and_export(
    host: &HostWgpuContext,
    width: u32,
    height: u32,
    color: [f32; 4],
) -> ExportedDmabuf {
    let (vk_device, vk_instance, physical_device, queue_family) = unsafe {
        let hal_device = host
            .device
            .as_hal::<wgpu::wgc::api::Vulkan>()
            .expect("Vulkan hal device");
        (
            hal_device.raw_device().clone(),
            hal_device.shared_instance().raw_instance().clone(),
            hal_device.raw_physical_device(),
            hal_device.queue_family_index(),
        )
    };
    let vk_queue = unsafe {
        host.queue
            .as_hal::<wgpu::wgc::api::Vulkan>()
            .expect("Vulkan hal queue")
            .as_raw()
    };

    let modifiers = [DRM_FORMAT_MOD_LINEAR];
    let mut modifier_list = vk::ImageDrmFormatModifierListCreateInfoEXT::default()
        .drm_format_modifiers(&modifiers);
    let mut external_mem_info = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_UNORM)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut external_mem_info)
        .push_next(&mut modifier_list);

    let image = unsafe {
        vk_device
            .create_image(&image_info, None)
            .expect("create_image")
    };

    let mem_reqs = unsafe { vk_device.get_image_memory_requirements(image) };
    let mem_props = unsafe { vk_instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = (0..mem_props.memory_type_count)
        .find(|&i| {
            (mem_reqs.memory_type_bits & (1 << i)) != 0
                && mem_props.memory_types[i as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
        })
        .expect("no DEVICE_LOCAL memory type compatible with image");

    let mut export_info = vk::ExportMemoryAllocateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let mut dedicated_info = vk::MemoryDedicatedAllocateInfo::default().image(image);

    let memory = unsafe {
        vk_device
            .allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(mem_reqs.size)
                    .memory_type_index(memory_type_index)
                    .push_next(&mut export_info)
                    .push_next(&mut dedicated_info),
                None,
            )
            .expect("allocate_memory")
    };

    unsafe {
        vk_device
            .bind_image_memory(image, memory, 0)
            .expect("bind_image_memory");
    }

    let drm_modifier_api =
        ash::ext::image_drm_format_modifier::Device::new(&vk_instance, &vk_device);
    let mut modifier_props = vk::ImageDrmFormatModifierPropertiesEXT::default();
    unsafe {
        drm_modifier_api
            .get_image_drm_format_modifier_properties(image, &mut modifier_props)
            .expect("get_image_drm_format_modifier_properties");
    }

    let subres = vk::ImageSubresource::default()
        .aspect_mask(vk::ImageAspectFlags::MEMORY_PLANE_0_EXT)
        .mip_level(0)
        .array_layer(0);
    let layout = unsafe { vk_device.get_image_subresource_layout(image, subres) };

    let cmd_pool = unsafe {
        vk_device
            .create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(queue_family)
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT),
                None,
            )
            .expect("create_command_pool")
    };
    let cmd = unsafe {
        vk_device
            .allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(cmd_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
            .expect("allocate_command_buffers")[0]
    };

    let subresource_range = vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .base_mip_level(0)
        .level_count(1)
        .base_array_layer(0)
        .layer_count(1);

    unsafe {
        vk_device
            .begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
            .expect("begin_command_buffer");

        let pre_clear_barrier = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(subresource_range);
        vk_device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[pre_clear_barrier],
        );

        let clear_value = vk::ClearColorValue { float32: color };
        vk_device.cmd_clear_color_image(
            cmd,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &clear_value,
            &[subresource_range],
        );

        vk_device
            .end_command_buffer(cmd)
            .expect("end_command_buffer");

        let cmd_buffers = [cmd];
        vk_device
            .queue_submit(
                vk_queue,
                &[vk::SubmitInfo::default().command_buffers(&cmd_buffers)],
                vk::Fence::null(),
            )
            .expect("queue_submit");
        vk_device
            .queue_wait_idle(vk_queue)
            .expect("queue_wait_idle");
    }

    let ext_mem_fd = ash::khr::external_memory_fd::Device::new(&vk_instance, &vk_device);
    let fd = unsafe {
        ext_mem_fd
            .get_memory_fd(
                &vk::MemoryGetFdInfoKHR::default()
                    .memory(memory)
                    .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT),
            )
            .expect("get_memory_fd")
    };

    ExportedDmabuf {
        fd,
        offset: layout.offset,
        row_pitch: layout.row_pitch,
        modifier: modifier_props.drm_format_modifier,
        _producer_state: ProducerState {
            vk_device,
            image,
            memory,
            cmd_pool,
        },
    }
}

fn readback_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    const BYTES_PER_PIXEL: u32 = 4;
    let unpadded_bpr = width * BYTES_PER_PIXEL;
    let padded_bpr = unpadded_bpr.div_ceil(256) * 256;

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("dmabuf-readback"),
        size: (padded_bpr * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("dmabuf-readback-encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bpr),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        tx.send(res).expect("send map_async result");
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    rx.recv().expect("recv map_async").expect("map_async error");

    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((unpadded_bpr * height) as usize);
    for row in 0..height as usize {
        let start = row * padded_bpr as usize;
        out.extend_from_slice(&data[start..start + unpadded_bpr as usize]);
    }
    drop(data);
    buffer.unmap();
    out
}
