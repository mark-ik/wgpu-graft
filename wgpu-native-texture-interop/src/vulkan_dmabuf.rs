//! DMABUF → Vulkan → wgpu import path for Linux producers.
//!
//! Imports a Linux DMABUF (e.g. from a WPE WebView, KMS, or other DRM
//! producer) directly into a `wgpu::Texture` on the host's wgpu Vulkan
//! device, using `VK_KHR_external_memory_fd` together with
//! `VK_EXT_image_drm_format_modifier` for tile/format-modifier-aware
//! imports.
//!
//! Unlike [`crate::raw_gl::linux`], this path does not bridge through GL
//! at all — the producer hands the consumer a DMABUF fd directly, and the
//! importer wraps it as a Vulkan image bound to externally-imported
//! memory.

#[cfg(target_os = "linux")]
use ash::vk;

use crate::{HostWgpuContext, InteropError, VulkanExternalImage};

#[cfg(target_os = "linux")]
fn map_format(format: wgpu::TextureFormat) -> Result<vk::Format, InteropError> {
    match format {
        wgpu::TextureFormat::Rgba8Unorm => Ok(vk::Format::R8G8B8A8_UNORM),
        wgpu::TextureFormat::Rgba8UnormSrgb => Ok(vk::Format::R8G8B8A8_SRGB),
        wgpu::TextureFormat::Bgra8Unorm => Ok(vk::Format::B8G8R8A8_UNORM),
        wgpu::TextureFormat::Bgra8UnormSrgb => Ok(vk::Format::B8G8R8A8_SRGB),
        other => Err(InteropError::Vulkan(format!(
            "VulkanExternalImage import does not yet support format {:?}",
            other
        ))),
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn import_vulkan_external_image(
    frame: &VulkanExternalImage,
    host: &HostWgpuContext,
) -> Result<wgpu::Texture, InteropError> {
    if frame.dmabuf_fd <= 0 {
        return Err(InteropError::InvalidFrame("dmabuf_fd <= 0"));
    }
    if host.backend != crate::InteropBackend::Vulkan {
        return Err(InteropError::BackendMismatch {
            expected: "Vulkan",
            actual: "non-Vulkan",
        });
    }

    let vk_format = map_format(frame.format)?;
    let extent = vk::Extent3D {
        width: frame.size.width,
        height: frame.size.height,
        depth: 1,
    };

    unsafe {
        let hal_device = host.device.as_hal::<wgpu::wgc::api::Vulkan>().ok_or(
            InteropError::BackendMismatch {
                expected: "Vulkan",
                actual: "non-Vulkan",
            },
        )?;
        let vk_device = hal_device.raw_device().clone();
        let vk_instance = hal_device.shared_instance().raw_instance().clone();
        let physical_device = hal_device.raw_physical_device();

        let plane_layouts = [vk::SubresourceLayout {
            offset: frame.dmabuf_offset,
            size: 0,
            row_pitch: frame.dmabuf_stride,
            array_pitch: 0,
            depth_pitch: 0,
        }];
        let mut drm_modifier_info = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
            .drm_format_modifier(frame.drm_modifier)
            .plane_layouts(&plane_layouts);
        let mut external_memory_info = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);

        let image_create_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk_format)
            .extent(extent)
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .push_next(&mut external_memory_info)
            .push_next(&mut drm_modifier_info);

        let vulkan_image = vk_device
            .create_image(&image_create_info, None)
            .map_err(|err| InteropError::Vulkan(format!("create_image (dmabuf): {}", err)))?;

        let memory = match allocate_and_bind_dmabuf_memory(
            &vk_device,
            &vk_instance,
            physical_device,
            vulkan_image,
            frame.dmabuf_fd,
        ) {
            Ok(memory) => memory,
            Err(err) => {
                vk_device.destroy_image(vulkan_image, None);
                return Err(err);
            }
        };

        let vk_device_for_drop = vk_device.clone();
        let imported = host
            .device
            .create_texture_from_hal::<wgpu::wgc::api::Vulkan>(
                hal_device.texture_from_raw(
                    vulkan_image,
                    &wgpu_hal::TextureDescriptor {
                        label: Some("dmabuf-vulkan-import"),
                        size: wgpu::Extent3d {
                            width: frame.size.width,
                            height: frame.size.height,
                            depth_or_array_layers: 1,
                        },
                        format: frame.format,
                        dimension: wgpu::TextureDimension::D2,
                        mip_level_count: 1,
                        sample_count: 1,
                        usage: wgpu::TextureUses::RESOURCE,
                        view_formats: Vec::new(),
                        memory_flags: wgpu_hal::MemoryFlags::empty(),
                    },
                    Some(Box::new(move || {
                        vk_device_for_drop.destroy_image(vulkan_image, None);
                        vk_device_for_drop.free_memory(memory, None);
                    })),
                    wgpu_hal::vulkan::TextureMemory::External,
                ),
                &wgpu::TextureDescriptor {
                    label: Some("dmabuf-vulkan-import"),
                    size: wgpu::Extent3d {
                        width: frame.size.width,
                        height: frame.size.height,
                        depth_or_array_layers: 1,
                    },
                    format: frame.format,
                    dimension: wgpu::TextureDimension::D2,
                    mip_level_count: 1,
                    sample_count: 1,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            );

        Ok(imported)
    }
}

#[cfg(target_os = "linux")]
unsafe fn allocate_and_bind_dmabuf_memory(
    vk_device: &ash::Device,
    vk_instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    vulkan_image: vk::Image,
    dmabuf_fd: i32,
) -> Result<vk::DeviceMemory, InteropError> {
    let external_memory_fd_api =
        ash::khr::external_memory_fd::Device::new(vk_instance, vk_device);

    let mut fd_properties = vk::MemoryFdPropertiesKHR::default();
    unsafe {
        external_memory_fd_api
            .get_memory_fd_properties(
                vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
                dmabuf_fd,
                &mut fd_properties,
            )
            .map_err(|err| {
                InteropError::Vulkan(format!("get_memory_fd_properties: {}", err))
            })?;
    }

    let memory_requirements = unsafe { vk_device.get_image_memory_requirements(vulkan_image) };
    let allowed_memory_type_bits =
        memory_requirements.memory_type_bits & fd_properties.memory_type_bits;
    let memory_properties =
        unsafe { vk_instance.get_physical_device_memory_properties(physical_device) };
    let memory_type_index = memory_properties.memory_types
        [..memory_properties.memory_type_count as usize]
        .iter()
        .enumerate()
        .position(|(i, _)| (allowed_memory_type_bits & (1 << i)) != 0)
        .ok_or_else(|| {
            InteropError::Vulkan("no memory type compatible with dmabuf import".into())
        })? as u32;

    let mut import_memory_info = vk::ImportMemoryFdInfoKHR::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
        .fd(dmabuf_fd);
    let mut dedicated_allocate_info =
        vk::MemoryDedicatedAllocateInfo::default().image(vulkan_image);

    let memory = unsafe {
        vk_device
            .allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(memory_requirements.size)
                    .memory_type_index(memory_type_index)
                    .push_next(&mut import_memory_info)
                    .push_next(&mut dedicated_allocate_info),
                None,
            )
            .map_err(|err| {
                InteropError::Vulkan(format!("allocate_memory (dmabuf import): {}", err))
            })?
    };

    if let Err(err) = unsafe { vk_device.bind_image_memory(vulkan_image, memory, 0) } {
        unsafe { vk_device.free_memory(memory, None) };
        return Err(InteropError::Vulkan(format!("bind_image_memory: {}", err)));
    }

    Ok(memory)
}
