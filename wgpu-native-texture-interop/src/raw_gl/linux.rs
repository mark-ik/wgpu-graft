//! GL framebuffer import via Vulkan external memory (Linux/Android).
//!
//! Imports a GL framebuffer into a `wgpu::Texture` by:
//! 1. Creating a Vulkan image with external memory (opaque FD)
//! 2. Importing the FD into GL via `GL_EXT_memory_object_fd`
//! 3. Blitting from the source FBO to the shared GL texture
//! 4. Wrapping the Vulkan image as a `wgpu::Texture`

use ash::vk;
use glow::HasContext;
use std::ffi::c_void;
use dpi::PhysicalSize;

use crate::{HostWgpuContext, InteropError, gl_bindings as gl};

/// Import a GL framebuffer into a `wgpu::Texture` via Vulkan external memory.
///
/// # Arguments
///
/// * `gl` - A glow GL context for texture and framebuffer operations
/// * `gl_extension_loader` - Function to load GL extension entry points (e.g.
///   `GL_EXT_memory_object_fd`). Typically wraps `eglGetProcAddress` or equivalent.
/// * `source_fbo` - The GL framebuffer object to read from. Pass 0 for the default framebuffer.
/// * `size` - Dimensions of the framebuffer content to import.
/// * `host` - The host wgpu context (device + queue) that will own the resulting texture.
///
/// The returned texture is `Rgba8Unorm` with top-left origin (Y-flipped during blit).
pub fn import_gl_framebuffer_vulkan(
    gl: &glow::Context,
    gl_extension_loader: &dyn Fn(&str) -> *const c_void,
    source_fbo: u32,
    size: PhysicalSize<u32>,
    host: &HostWgpuContext,
) -> Result<wgpu::Texture, InteropError> {
    use gl::Gles2 as Gl;

    unsafe {
        let hal_device = host.device.as_hal::<wgpu::wgc::api::Vulkan>().ok_or(
            InteropError::BackendMismatch {
                expected: "Vulkan",
                actual: "non-Vulkan",
            },
        )?;
        let vulkan_device = hal_device.raw_device().clone();
        let vulkan_instance = hal_device.shared_instance().raw_instance();
        let physical_device = hal_device.raw_physical_device();

        let mut external_memory_image_info = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);

        let vulkan_image = vulkan_device
            .create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(vk::Format::R8G8B8A8_UNORM)
                    .extent(vk::Extent3D {
                        width: size.width,
                        height: size.height,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::COLOR_ATTACHMENT)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .push_next(&mut external_memory_image_info),
                None,
            )
            .map_err(|err| InteropError::Vulkan(err.to_string()))?;

        let memory_requirements = vulkan_device.get_image_memory_requirements(vulkan_image);
        let memory_properties =
            vulkan_instance.get_physical_device_memory_properties(physical_device);
        let memory_type_index = memory_properties.memory_types
            [..memory_properties.memory_type_count as usize]
            .iter()
            .enumerate()
            .position(|(i, mem_type)| {
                (memory_requirements.memory_type_bits & (1 << i)) != 0
                    && mem_type
                        .property_flags
                        .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .ok_or_else(|| {
                InteropError::Vulkan("no DEVICE_LOCAL memory type compatible with image".into())
            })? as u32;

        let mut dedicated_allocate_info =
            vk::MemoryDedicatedAllocateInfo::default().image(vulkan_image);
        let mut export_info = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);

        let memory = vulkan_device
            .allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(memory_requirements.size)
                    .memory_type_index(memory_type_index)
                    .push_next(&mut dedicated_allocate_info)
                    .push_next(&mut export_info),
                None,
            )
            .map_err(|err| InteropError::Vulkan(err.to_string()))?;

        vulkan_device
            .bind_image_memory(vulkan_image, memory, 0)
            .map_err(|err| InteropError::Vulkan(err.to_string()))?;

        let external_memory_fd_api =
            ash::khr::external_memory_fd::Device::new(&vulkan_instance, &vulkan_device);
        let memory_handle = external_memory_fd_api
            .get_memory_fd(
                &vk::MemoryGetFdInfoKHR::default()
                    .memory(memory)
                    .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD),
            )
            .map_err(|err| InteropError::Vulkan(err.to_string()))?;

        let gl_with_extensions = Gl::load_with(|function_name| gl_extension_loader(function_name));

        if !gl_with_extensions.CreateMemoryObjectsEXT.is_loaded()
            || !gl_with_extensions.ImportMemoryFdEXT.is_loaded()
            || !gl_with_extensions.TexStorageMem2DEXT.is_loaded()
        {
            vulkan_device.destroy_image(vulkan_image, None);
            vulkan_device.free_memory(memory, None);
            return Err(InteropError::OpenGl(
                "GL_EXT_memory_object_fd not available — the GL driver does not \
                 support external memory extensions"
                    .into(),
            ));
        }

        let mut memory_object = 0;
        gl_with_extensions.CreateMemoryObjectsEXT(1, &mut memory_object);
        gl_with_extensions.MemoryObjectParameterivEXT(
            memory_object,
            gl::DEDICATED_MEMORY_OBJECT_EXT,
            &1,
        );
        gl_with_extensions.ImportMemoryFdEXT(
            memory_object,
            memory_requirements.size,
            gl::HANDLE_TYPE_OPAQUE_FD_EXT,
            memory_handle,
        );

        let texture = gl.create_texture().map_err(InteropError::OpenGl)?;
        gl.bind_texture(gl::TEXTURE_2D, Some(texture));
        gl_with_extensions.TexStorageMem2DEXT(
            gl::TEXTURE_2D,
            1,
            gl::RGBA8,
            size.width as i32,
            size.height as i32,
            memory_object,
            0,
        );

        let draw_framebuffer = gl.create_framebuffer().map_err(InteropError::OpenGl)?;

        let read_framebuffer = if source_fbo == 0 {
            None
        } else {
            Some(
                glow::NativeFramebuffer(
                    std::num::NonZeroU32::new(source_fbo)
                        .ok_or(InteropError::InvalidFrame("invalid FBO id"))?,
                ),
            )
        };

        gl.bind_framebuffer(gl::DRAW_FRAMEBUFFER, Some(draw_framebuffer));
        gl.framebuffer_texture_2d(
            gl::DRAW_FRAMEBUFFER,
            gl::COLOR_ATTACHMENT0,
            gl::TEXTURE_2D,
            Some(texture),
            0,
        );

        gl.bind_framebuffer(gl::READ_FRAMEBUFFER, read_framebuffer);
        gl.bind_framebuffer(gl::DRAW_FRAMEBUFFER, Some(draw_framebuffer));
        gl.blit_framebuffer(
            0,
            0,
            size.width as i32,
            size.height as i32,
            0,
            size.height as i32,
            size.width as i32,
            0,
            gl::COLOR_BUFFER_BIT,
            gl::NEAREST,
        );
        gl.flush();
        gl.delete_framebuffer(draw_framebuffer);
        gl.delete_texture(texture);
        gl_with_extensions.DeleteMemoryObjectsEXT(1, &memory_object);

        let imported = host
            .device
            .create_texture_from_hal::<wgpu::wgc::api::Vulkan>(
                hal_device.texture_from_raw(
                    vulkan_image,
                    &wgpu_hal::TextureDescriptor {
                        label: None,
                        size: wgpu::Extent3d {
                            width: size.width,
                            height: size.height,
                            depth_or_array_layers: 1,
                        },
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        dimension: wgpu::TextureDimension::D2,
                        mip_level_count: 1,
                        sample_count: 1,
                        usage: wgpu::TextureUses::RESOURCE | wgpu::TextureUses::COLOR_TARGET,
                        view_formats: Vec::new(),
                        memory_flags: wgpu_hal::MemoryFlags::empty(),
                    },
                    Some(Box::new(move || {
                        vulkan_device.destroy_image(vulkan_image, None);
                        vulkan_device.free_memory(memory, None);
                    })),
                    wgpu_hal::vulkan::TextureMemory::External,
                ),
                &wgpu::TextureDescriptor {
                    label: Some("gl-frame-vulkan-import"),
                    size: wgpu::Extent3d {
                        width: size.width,
                        height: size.height,
                        depth_or_array_layers: 1,
                    },
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    dimension: wgpu::TextureDimension::D2,
                    mip_level_count: 1,
                    sample_count: 1,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
            );

        Ok(imported)
    }
}
