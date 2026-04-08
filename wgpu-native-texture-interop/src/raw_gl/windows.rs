//! GL framebuffer import via Vulkan external memory with NT handles (Windows).
//!
//! Imports a GL framebuffer into a `wgpu::Texture` by:
//! 1. Creating a Vulkan image with external memory (opaque Win32 handle)
//! 2. Exporting an NT handle from the Vulkan memory allocation
//! 3. Importing the NT handle into GL via `GL_EXT_memory_object_win32`
//! 4. Blitting from the source FBO to the shared GL texture
//! 5. Wrapping the Vulkan image as a `wgpu::Texture`
//!
//! This is the Windows equivalent of `raw_gl::linux::import_gl_framebuffer_vulkan`.
//! Requires the host wgpu device to use the Vulkan backend (not DX12).

use ash::vk;
use glow::HasContext;
use std::ffi::c_void;
use winit::dpi::PhysicalSize;

use crate::{HostWgpuContext, InteropError, gl_bindings as gl};

/// Import a GL framebuffer into a `wgpu::Texture` via Vulkan external memory (NT handles).
///
/// # Arguments
///
/// * `gl` - A glow GL context for texture and framebuffer operations
/// * `gl_extension_loader` - Function to load GL extension entry points (e.g.
///   wrapping `wglGetProcAddress`). Must support `GL_EXT_memory_object_win32`.
/// * `source_fbo` - The GL framebuffer object to read from. Pass 0 for the default framebuffer.
/// * `size` - Dimensions of the framebuffer content to import.
/// * `host` - The host wgpu context (must be Vulkan backend on Windows).
///
/// # Requirements
///
/// - The wgpu device must be using Vulkan (not DX12). Request Vulkan explicitly:
///   ```ignore
///   let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
///       backends: wgpu::Backends::VULKAN,
///       ..Default::default()
///   });
///   ```
/// - Vulkan drivers must support `VK_KHR_external_memory_win32`
/// - The GL context must support `GL_EXT_memory_object` and `GL_EXT_memory_object_win32`
///
/// The returned texture is `Rgba8Unorm` with top-left origin (Y-flipped during blit).
pub fn import_gl_framebuffer_vulkan_win32(
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

        // Create Vulkan image with Win32 external memory handle type
        let mut external_memory_image_info = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32);

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

        // Allocate memory with dedicated allocation and Win32 export
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
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32);

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

        // Export NT handle from Vulkan memory
        let external_memory_win32_api =
            ash::khr::external_memory_win32::Device::new(&vulkan_instance, &vulkan_device);
        let memory_handle = external_memory_win32_api
            .get_memory_win32_handle(
                &vk::MemoryGetWin32HandleInfoKHR::default()
                    .memory(memory)
                    .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32),
            )
            .map_err(|err| InteropError::Vulkan(err.to_string()))?;

        // Import NT handle into GL via GL_EXT_memory_object_win32
        let gl_with_extensions = Gl::load_with(|function_name| gl_extension_loader(function_name));

        // Verify the GL driver exposes the required extensions. ANGLE's default
        // D3D11 backend does NOT support GL_EXT_memory_object_win32, so these
        // function pointers will be panic stubs unless a Vulkan-backed GL driver
        // (e.g. Mesa, NVIDIA, AMD) is in use.
        if !gl_with_extensions.CreateMemoryObjectsEXT.is_loaded()
            || !gl_with_extensions.ImportMemoryWin32HandleEXT.is_loaded()
            || !gl_with_extensions.TexStorageMem2DEXT.is_loaded()
        {
            // Clean up the Vulkan resources we already allocated.
            vulkan_device.destroy_image(vulkan_image, None);
            vulkan_device.free_memory(memory, None);
            return Err(InteropError::OpenGl(
                "GL_EXT_memory_object_win32 not available (ANGLE's D3D11 backend \
                 does not support external memory extensions — use CPU readback instead)"
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
        gl_with_extensions.ImportMemoryWin32HandleEXT(
            memory_object,
            memory_requirements.size,
            gl::HANDLE_TYPE_OPAQUE_WIN32_EXT,
            memory_handle as *mut c_void,
        );

        // Create GL texture backed by the shared Vulkan memory
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

        // Blit from source FBO to shared texture (with Y-flip)
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

        // Clean up GL resources (Vulkan image/memory stay alive via wgpu)
        gl.delete_framebuffer(draw_framebuffer);
        gl.delete_texture(texture);
        gl_with_extensions.DeleteMemoryObjectsEXT(1, &memory_object);

        // Wrap the Vulkan image as a wgpu texture
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
                    label: Some("gl-frame-vulkan-win32-import"),
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
