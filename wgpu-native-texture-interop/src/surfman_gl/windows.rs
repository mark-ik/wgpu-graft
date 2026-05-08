use crate::{
    GlFramebufferSource, HostWgpuContext, ImportOptions, ImportedTexture, InteropError,
    SyncMechanism, TextureOrigin,
};

use super::SurfmanGlFrameSource;

pub(super) fn import_current_frame(
    source: &SurfmanGlFrameSource,
    frame: &GlFramebufferSource,
    host: &HostWgpuContext,
    _options: &ImportOptions,
) -> Result<ImportedTexture, InteropError> {
    let device = &source.context.device.borrow();
    let mut context = source.context.context.borrow_mut();

    // Make the context current WITH the surface still bound so that
    // eglGetCurrentSurface(EGL_DRAW) returns the ANGLE pbuffer.
    // Servo may have released the context after rendering; this restores it.
    device
        .make_context_current(&mut context)
        .map_err(|err| InteropError::Surfman(format!("{err:?}")))?;

    // ── Fast path: ANGLE D3D11 share handle (zero-copy, no GL extension needed) ──
    // The context is now current with the pbuffer as the draw surface, so
    // eglQuerySurfacePointerANGLE can retrieve the backing D3D11 handle.
    match crate::raw_gl::angle_d3d11::import_angle_d3d11_frame(source.size, host) {
        Ok(texture) => {
            return Ok(ImportedTexture {
                texture,
                format: wgpu::TextureFormat::Bgra8Unorm,
                size: frame.size(),
                origin: TextureOrigin::TopLeft,
                generation: source.generation,
                consumer_sync: SyncMechanism::ImplicitGlFlush,
            });
        }
        Err(_) => {
            // Not an ANGLE context, or Vulkan backend unavailable; try GL extension path.
        }
    }

    // ── Slow path: GL_EXT_memory_object_win32 (non-ANGLE Vulkan GL) ──────────────
    let surface = device
        .unbind_surface_from_context(&mut context)
        .map_err(|err| InteropError::Surfman(format!("{err:?}")))?
        .ok_or(InteropError::InvalidFrame("no surfman surface available"))?;

    device
        .make_context_current(&mut context)
        .map_err(|err| InteropError::Surfman(format!("{err:?}")))?;

    let surface_info = device.surface_info(&surface);
    let source_fbo = surface_info
        .framebuffer_object
        .map(|fb| fb.0.get())
        .unwrap_or(0);

    let result = crate::raw_gl::windows::import_gl_framebuffer_vulkan_win32(
        &source.context.glow_gl,
        &|name| device.get_proc_address(&context, name),
        source_fbo,
        source.size,
        host,
    );

    let _ = device
        .bind_surface_to_context(&mut context, surface)
        .map_err(|(err, mut surface)| {
            let _ = device.destroy_surface(&mut context, &mut surface);
            err
        });

    result.map(|texture| ImportedTexture {
        texture,
        format: wgpu::TextureFormat::Rgba8Unorm,
        size: frame.size(),
        origin: TextureOrigin::TopLeft,
        generation: source.generation,
        consumer_sync: SyncMechanism::ImplicitGlFlush,
    })
}

/// Import the current surfman frame into a `wgpu::Texture` on a DX12 host.
///
/// Tries paths in order:
///
/// 1. **ANGLE D3D11 → DX12 shared texture** ([`super::windows_dx12_shared`])
///    — works against Servo's default ANGLE-D3D11 surfman backend by
///    allocating the shared texture on ANGLE's own D3D11 device and wrapping
///    it as a transient EGL pbuffer. Fast path.
/// 2. **`GL_EXT_memory_object_win32`** ([`crate::raw_gl::dx12`]) — exports a
///    DX12 resource into GL via external-memory extensions. ANGLE does not
///    expose these extensions, so this path only succeeds against non-ANGLE
///    Vulkan-backed surfman GL contexts.
pub(super) fn import_current_frame_dx12(
    source: &SurfmanGlFrameSource,
    frame: &GlFramebufferSource,
    host: &HostWgpuContext,
    _options: &ImportOptions,
) -> Result<ImportedTexture, InteropError> {
    let device = &source.context.device.borrow();
    let mut context = source.context.context.borrow_mut();

    // Make the context current so EGL queries (and the ANGLE D3D11-shared path)
    // see the right surface.
    device
        .make_context_current(&mut context)
        .map_err(|err| InteropError::Surfman(format!("{err:?}")))?;

    // ── Fast path: ANGLE D3D11 shared NT-handle → wgpu DX12 ──────────────────
    let bound_fbo = surface_fbo(device, &context);
    match super::windows_dx12_shared::import_current_frame(
        &source.angle_dx12_shared,
        device,
        &mut context,
        &source.context.glow_gl,
        bound_fbo,
        source.size,
        host,
    ) {
        Ok(texture) => {
            return Ok(ImportedTexture {
                texture,
                format: wgpu::TextureFormat::Rgba8Unorm,
                size: frame.size(),
                origin: TextureOrigin::TopLeft,
                generation: source.generation,
                consumer_sync: SyncMechanism::ImplicitGlFlush,
            });
        }
        Err(_) => {
            // Not an ANGLE D3D11 surfman context, or DX12 device unavailable;
            // try the GL_EXT_memory_object_win32 path against an unbound surface.
        }
    }

    // ── Slow path: GL_EXT_memory_object_win32 (non-ANGLE Vulkan GL) ──────────
    let surface = device
        .unbind_surface_from_context(&mut context)
        .map_err(|err| InteropError::Surfman(format!("{err:?}")))?
        .ok_or(InteropError::InvalidFrame("no surfman surface available"))?;

    device
        .make_context_current(&mut context)
        .map_err(|err| InteropError::Surfman(format!("{err:?}")))?;

    let surface_info = device.surface_info(&surface);
    let source_fbo = surface_info
        .framebuffer_object
        .map(|fb| fb.0.get())
        .unwrap_or(0);

    let result = crate::raw_gl::dx12::import_gl_framebuffer_dx12(
        &source.context.glow_gl,
        &|name| device.get_proc_address(&context, name),
        source_fbo,
        source.size,
        host,
    );

    let _ = device
        .bind_surface_to_context(&mut context, surface)
        .map_err(|(err, mut surface)| {
            let _ = device.destroy_surface(&mut context, &mut surface);
            err
        });

    result.map(|texture| ImportedTexture {
        texture,
        format: wgpu::TextureFormat::Rgba8Unorm,
        size: frame.size(),
        origin: TextureOrigin::TopLeft,
        generation: source.generation,
        consumer_sync: SyncMechanism::ImplicitGlFlush,
    })
}

/// Read the current surfman surface's GL framebuffer object id without
/// unbinding the surface. Returns `0` (the default framebuffer) if no
/// surface-backed framebuffer is reported.
fn surface_fbo(device: &surfman::Device, context: &surfman::Context) -> u32 {
    device
        .context_surface_info(context)
        .ok()
        .flatten()
        .and_then(|info| info.framebuffer_object)
        .map(|fb| fb.0.get())
        .unwrap_or(0)
}
