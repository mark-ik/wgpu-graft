//! GL framebuffer import via D3D12 shared texture (Windows).
//!
//! Imports a GL framebuffer into a `wgpu::Texture` by:
//! 1. Creating a D3D12 texture with `ALLOW_SIMULTANEOUS_ACCESS` and `HEAP_FLAG_SHARED`
//! 2. Exporting a DXGI NT shared handle via `ID3D12Device::CreateSharedHandle`
//! 3. Importing the NT handle into GL via `GL_EXT_memory_object_win32`
//!    using `GL_HANDLE_TYPE_D3D12_RESOURCE_EXT`
//! 4. Blitting from the source FBO to the shared GL texture (with Y-flip)
//! 5. Wrapping the D3D12 resource as a `wgpu::Texture`
//!
//! Use this path when the host wgpu device uses the D3D12 backend. For
//! Vulkan on Windows, use [`super::windows::import_gl_framebuffer_vulkan_win32`].

use glow::HasContext;
use std::ffi::c_void;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Graphics::Direct3D12::{
    D3D12_HEAP_FLAG_SHARED, D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_DEFAULT,
    D3D12_RESOURCE_DESC, D3D12_RESOURCE_DIMENSION_TEXTURE2D,
    D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET, D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
    D3D12_RESOURCE_STATE_COMMON, D3D12_TEXTURE_LAYOUT_UNKNOWN, ID3D12Resource,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
};
use dpi::PhysicalSize;

use crate::{HostWgpuContext, InteropError, gl_bindings as gl};

/// Import a GL framebuffer into a `wgpu::Texture` via a D3D12 shared texture.
///
/// # Arguments
///
/// * `gl` - A glow GL context for texture and framebuffer operations
/// * `gl_extension_loader` - Function to load GL extension entry points (e.g.
///   wrapping `wglGetProcAddress`). Must support `GL_EXT_memory_object_win32`.
/// * `source_fbo` - The GL framebuffer object to read from. Pass 0 for the default framebuffer.
/// * `size` - Dimensions of the framebuffer content to import.
/// * `host` - The host wgpu context (must be D3D12 backend).
///
/// # Requirements
///
/// - The wgpu device must be using D3D12. Request D3D12 explicitly or set
///   `WGPU_BACKEND=dx12` in the environment.
/// - The GL context must support `GL_EXT_memory_object` and `GL_EXT_memory_object_win32`.
/// - Drivers must support `GL_HANDLE_TYPE_D3D12_RESOURCE_EXT` (all major vendors on Windows).
///
/// The returned texture is `Rgba8Unorm` with top-left origin (Y-flipped during blit).
pub fn import_gl_framebuffer_dx12(
    gl: &glow::Context,
    gl_extension_loader: &dyn Fn(&str) -> *const c_void,
    source_fbo: u32,
    size: PhysicalSize<u32>,
    host: &HostWgpuContext,
) -> Result<wgpu::Texture, InteropError> {
    use gl::Gles2 as Gl;

    unsafe {
        let hal_device = host
            .device
            .as_hal::<wgpu::wgc::api::Dx12>()
            .ok_or(InteropError::BackendMismatch {
                expected: "Dx12",
                actual: "non-Dx12",
            })?;
        let d3d_device = hal_device.raw_device().clone();

        // Create a D3D12 texture that can be shared with GL via a DXGI NT handle.
        // ALLOW_SIMULTANEOUS_ACCESS lets GL and D3D12 access the resource without
        // explicit resource-state transitions between APIs.
        // HEAP_FLAG_SHARED is required to call ID3D12Device::CreateSharedHandle.
        let heap_props = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            ..Default::default()
        };
        let resource_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Width: size.width as u64,
            Height: size.height,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET
                | D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
            ..Default::default()
        };

        let mut d3d12_resource: Option<ID3D12Resource> = None;
        d3d_device
            .CreateCommittedResource(
                &heap_props,
                D3D12_HEAP_FLAG_SHARED,
                &resource_desc,
                D3D12_RESOURCE_STATE_COMMON,
                None,
                &mut d3d12_resource,
            )
            .map_err(|e| InteropError::Dx12(e.to_string()))?;
        let d3d12_resource = d3d12_resource
            .ok_or_else(|| InteropError::Dx12("CreateCommittedResource returned null".into()))?;

        // GetResourceAllocationInfo returns the driver-padded size (aligned to 64 KB).
        // This must match the `size` parameter passed to ImportMemoryWin32HandleEXT.
        let alloc_info = d3d_device.GetResourceAllocationInfo(0, &[resource_desc]);

        // Export an NT shared handle from the D3D12 resource so GL can import it.
        let shared_handle = d3d_device
            .CreateSharedHandle(
                &d3d12_resource,
                None,
                (DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE).0,
                windows::core::PCWSTR::null(),
            )
            .map_err(|e| InteropError::Dx12(e.to_string()))?;

        // Import the NT handle into GL via GL_EXT_memory_object_win32.
        let gl_with_extensions = Gl::load_with(|function_name| gl_extension_loader(function_name));

        if !gl_with_extensions.CreateMemoryObjectsEXT.is_loaded()
            || !gl_with_extensions.ImportMemoryWin32HandleEXT.is_loaded()
            || !gl_with_extensions.TexStorageMem2DEXT.is_loaded()
        {
            let _ = CloseHandle(shared_handle);
            return Err(InteropError::OpenGl(
                "GL_EXT_memory_object_win32 not available (ANGLE's D3D11 backend \
                 does not support external memory extensions — use CPU readback instead)"
                    .into(),
            ));
        }

        let mut memory_object = 0u32;
        gl_with_extensions.CreateMemoryObjectsEXT(1, &mut memory_object);
        gl_with_extensions.ImportMemoryWin32HandleEXT(
            memory_object,
            alloc_info.SizeInBytes,
            gl::HANDLE_TYPE_D3D12_RESOURCE_EXT,
            shared_handle.0 as *mut c_void,
        );

        // The NT handle has been imported into GL; close our copy.
        let _ = CloseHandle(shared_handle);

        // Create a GL texture backed by the shared D3D12 memory.
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

        // Blit from the source FBO to the shared texture with a Y-flip so that
        // the resulting texture has top-left origin.
        let draw_framebuffer = gl.create_framebuffer().map_err(InteropError::OpenGl)?;

        let read_framebuffer = if source_fbo == 0 {
            None
        } else {
            Some(glow::NativeFramebuffer(
                std::num::NonZeroU32::new(source_fbo)
                    .ok_or(InteropError::InvalidFrame("invalid FBO id"))?,
            ))
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
            size.height as i32, // Y-flip: dst top = src bottom
            size.width as i32,
            0,
            gl::COLOR_BUFFER_BIT,
            gl::NEAREST,
        );
        gl.flush();

        // Clean up GL resources (the D3D12 resource stays alive via COM refcount in wgpu).
        gl.delete_framebuffer(draw_framebuffer);
        gl.delete_texture(texture);
        gl_with_extensions.DeleteMemoryObjectsEXT(1, &memory_object);

        // Wrap the D3D12 resource as a wgpu texture.
        // texture_from_raw is an associated function — no &self needed.
        // COM refcount on d3d12_resource manages the resource lifetime when wgpu
        // eventually calls destroy_texture.
        let hal_texture = wgpu_hal::dx12::Device::texture_from_raw(
            d3d12_resource,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureDimension::D2,
            wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            1, // mip_level_count
            1, // sample_count
        );

        let imported = host
            .device
            .create_texture_from_hal::<wgpu::wgc::api::Dx12>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("gl-frame-dx12-import"),
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
                        | wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                },
            );

        Ok(imported)
    }
}
