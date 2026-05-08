//! ANGLE D3D11 → DX12 shared-texture import path (Windows + DX12 host).
//!
//! Servo on Windows renders via ANGLE, an OpenGL ES implementation backed by
//! D3D11. ANGLE does not expose `GL_EXT_memory_object_win32`, so the
//! [`raw_gl::dx12`](crate::raw_gl::dx12) path that imports a D3D12 resource
//! into GL memory cannot work with Servo. This module takes the inverse
//! approach, adapted from slint-ui/slint examples/servo (PR #11089):
//!
//! 1. Allocate an `ID3D11Texture2D` with `D3D11_RESOURCE_MISC_SHARED |
//!    D3D11_RESOURCE_MISC_SHARED_NTHANDLE` on the **ANGLE D3D11 device**
//!    obtained from `surfman::Device::native_device().d3d11_device`.
//! 2. Wrap that texture as a transient EGL pbuffer surface via
//!    [`surfman::Device::create_surface_texture_from_texture`]. ANGLE will
//!    render into it through the regular GL pipeline.
//! 3. Open the same NT shared handle on the host wgpu DX12 device with
//!    [`ID3D12Device::OpenSharedHandle`] and wrap it as a `wgpu::Texture`
//!    via `wgpu_hal::dx12::Device::texture_from_raw`.
//! 4. On every frame: bind the EGL pbuffer, blit the source FBO into it
//!    (with Y-flip), drop the transient surface. The wgpu texture stays
//!    cached across frames; only re-create when the size changes.
//!
//! The size-dependent state is owned by [`AngleDx12SharedCache`] which is
//! shared from `SurfmanFrameProducer` into each `SurfmanGlFrameSource`.

use std::cell::RefCell;

use dpi::PhysicalSize;
use euclid::default::Size2D;
use glow::HasContext;

use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_RESOURCE_MISC_SHARED,
    D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Device,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Direct3D12::ID3D12Resource;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIResource1;
use windows::core::{IUnknown, Interface, PCWSTR};

use crate::{HostWgpuContext, InteropError};

/// One slot of the size-dependent shared-texture state.
///
/// Recreated whenever the producer's frame size changes; otherwise reused
/// across frames so the wgpu texture handle stays stable.
struct SizeDependentState {
    /// The shared D3D11 texture allocated on the ANGLE D3D11 device. Drives
    /// both the EGL pbuffer (for ANGLE/GL writes) and the wgpu DX12 texture
    /// (for host reads).
    d3d11_shared_texture: ID3D11Texture2D,
    /// The host-visible wgpu texture that aliases `d3d11_shared_texture`.
    wgpu_texture: wgpu::Texture,
    /// Cached so we can detect size changes without querying the texture
    /// descriptor each frame.
    size: PhysicalSize<u32>,
}

/// Cross-frame cache of the ANGLE-D3D11→DX12 shared texture state.
///
/// Holds a single size-keyed `SizeDependentState`. Owned by
/// `SurfmanFrameProducer` and cloned (`Rc`) into each emitted
/// `SurfmanGlFrameSource`.
pub(super) struct AngleDx12SharedCache {
    state: RefCell<Option<SizeDependentState>>,
}

impl AngleDx12SharedCache {
    pub(super) fn new() -> Self {
        Self {
            state: RefCell::new(None),
        }
    }
}

impl Default for AngleDx12SharedCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Import the current ANGLE EGL frame into a `wgpu::Texture` on a DX12 host.
///
/// Allocates (or reuses) a D3D11 shared texture on ANGLE's D3D11 device,
/// renders the source FBO into it via a transient EGL pbuffer, and returns
/// the host-visible wgpu DX12 texture aliasing the same memory.
///
/// # Errors
///
/// - [`InteropError::BackendMismatch`] if `host.device` is not running on the
///   DX12 backend.
/// - [`InteropError::Surfman`] if the ANGLE D3D11 device cannot be obtained
///   from the surfman device, or if the EGL pbuffer surface creation fails.
/// - [`InteropError::Dx12`] if D3D11 texture allocation, NT handle export, or
///   `OpenSharedHandle` on the wgpu DX12 device fails.
/// - [`InteropError::OpenGl`] if the FBO blit fails.
pub(super) fn import_current_frame(
    cache: &AngleDx12SharedCache,
    surfman_device: &surfman::Device,
    surfman_context: &mut surfman::Context,
    glow_gl: &glow::Context,
    source_fbo: u32,
    size: PhysicalSize<u32>,
    host: &HostWgpuContext,
) -> Result<wgpu::Texture, InteropError> {
    // Verify the wgpu device is running on DX12 before doing any allocation work.
    let _ = unsafe { host.device.as_hal::<wgpu::wgc::api::Dx12>() }.ok_or(
        InteropError::BackendMismatch {
            expected: "Dx12",
            actual: "non-Dx12",
        },
    )?;

    // (Re)create the size-dependent state if the producer frame size changed.
    let needs_recreate = cache
        .state
        .borrow()
        .as_ref()
        .map_or(true, |s| s.size != size);
    if needs_recreate {
        let d3d11_device = angle_d3d11_device(surfman_device)?;
        let new_state = init_size_dependent_state(&d3d11_device, size, &host.device)?;
        *cache.state.borrow_mut() = Some(new_state);
    }

    let cache_borrow = cache.state.borrow();
    let state = cache_borrow.as_ref().expect("just initialized");

    // Wrap the cached D3D11 texture as a transient EGL pbuffer for this
    // frame, blit the source FBO into it, then destroy the transient.
    let surface_texture = unsafe {
        let texture_size = Size2D::new(size.width as i32, size.height as i32);
        let raw = state.d3d11_shared_texture.clone().into_raw();
        let texture_comptr = wio::com::ComPtr::from_raw(raw as *mut _);

        surfman_device
            .create_surface_texture_from_texture(surfman_context, &texture_size, texture_comptr)
            .map_err(|err| {
                InteropError::Surfman(format!(
                    "create_surface_texture_from_texture failed: {err:?}"
                ))
            })?
    };

    let gl_texture = surfman_device
        .surface_texture_object(&surface_texture)
        .ok_or_else(|| InteropError::OpenGl("ANGLE returned no GL texture for pbuffer".into()))?;

    blit_fbo_to_gl_texture(glow_gl, source_fbo, gl_texture, size)?;

    // Tear down the transient pbuffer. The underlying D3D11 texture stays
    // alive via the cache's COM reference.
    let mut inner_surface = surfman_device
        .destroy_surface_texture(surfman_context, surface_texture)
        .map_err(|(err, _)| {
            InteropError::Surfman(format!("destroy_surface_texture failed: {err:?}"))
        })?;
    surfman_device
        .destroy_surface(surfman_context, &mut inner_surface)
        .map_err(|err| InteropError::Surfman(format!("destroy_surface failed: {err:?}")))?;

    Ok(state.wgpu_texture.clone())
}

/// Pull the underlying ANGLE D3D11 device pointer out of a surfman device and
/// wrap it as a `windows-rs` interface.
fn angle_d3d11_device(surfman_device: &surfman::Device) -> Result<ID3D11Device, InteropError> {
    let native_device = surfman_device.native_device();
    if native_device.d3d11_device.is_null() {
        return Err(InteropError::Surfman(
            "ANGLE D3D11 device pointer is null on surfman::Device::native_device()".into(),
        ));
    }
    unsafe {
        IUnknown::from_raw(native_device.d3d11_device as *mut _)
            .cast::<ID3D11Device>()
            .map_err(|err| InteropError::Dx12(format!("D3D11 device cast failed: {err}")))
    }
}

/// Allocate a new D3D11 shared texture on the ANGLE D3D11 device, export an NT
/// handle, open it on the wgpu DX12 device, and wrap as a `wgpu::Texture`.
fn init_size_dependent_state(
    d3d11_device: &ID3D11Device,
    size: PhysicalSize<u32>,
    wgpu_device: &wgpu::Device,
) -> Result<SizeDependentState, InteropError> {
    unsafe {
        // 1. Allocate the shared D3D11 texture.
        let mut d3d11_shared: Option<ID3D11Texture2D> = None;
        d3d11_device
            .CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: size.width,
                    Height: size.height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                    CPUAccessFlags: 0,
                    MiscFlags: (D3D11_RESOURCE_MISC_SHARED.0
                        | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0)
                        as u32,
                },
                None,
                Some(&mut d3d11_shared),
            )
            .map_err(|e| InteropError::Dx12(format!("D3D11 CreateTexture2D failed: {e}")))?;
        let d3d11_shared_texture = d3d11_shared.ok_or_else(|| {
            InteropError::Dx12("CreateTexture2D returned null texture".into())
        })?;

        // 2. Export an NT handle from the D3D11 texture.
        let dxgi_resource = d3d11_shared_texture
            .cast::<IDXGIResource1>()
            .map_err(|e| InteropError::Dx12(format!("Cast to IDXGIResource1 failed: {e}")))?;
        let nt_handle = dxgi_resource
            .CreateSharedHandle(None, GENERIC_ALL.0, PCWSTR::null())
            .map_err(|e| InteropError::Dx12(format!("DXGI CreateSharedHandle failed: {e}")))?;

        // 3. Open the handle on the wgpu DX12 device.
        let hal_device = wgpu_device
            .as_hal::<wgpu::wgc::api::Dx12>()
            .ok_or(InteropError::BackendMismatch {
                expected: "Dx12",
                actual: "non-Dx12",
            })?;
        let dx12_device = hal_device.raw_device().clone();

        let mut dx12_resource: Option<ID3D12Resource> = None;
        dx12_device
            .OpenSharedHandle(nt_handle, &mut dx12_resource)
            .map_err(|e| InteropError::Dx12(format!("D3D12 OpenSharedHandle failed: {e}")))?;
        let dx12_resource = dx12_resource
            .ok_or_else(|| InteropError::Dx12("OpenSharedHandle returned null resource".into()))?;

        // The NT handle has been opened on both sides; close our copy.
        let _ = CloseHandle(nt_handle);

        // 4. Wrap the DX12 resource as a wgpu texture.
        let extent = wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        };
        let hal_texture = wgpu_hal::dx12::Device::texture_from_raw(
            dx12_resource,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureDimension::D2,
            extent,
            1,
            1,
        );

        let wgpu_texture = wgpu_device.create_texture_from_hal::<wgpu::wgc::api::Dx12>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: Some("angle-d3d11-shared-dx12-import"),
                size: extent,
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

        Ok(SizeDependentState {
            d3d11_shared_texture,
            wgpu_texture,
            size,
        })
    }
}

/// Blit `source_fbo` into the GL texture backing the ANGLE EGL pbuffer, with
/// a Y-flip so the resulting image has top-left origin.
fn blit_fbo_to_gl_texture(
    gl: &glow::Context,
    source_fbo: u32,
    gl_texture: glow::Texture,
    size: PhysicalSize<u32>,
) -> Result<(), InteropError> {
    unsafe {
        let draw_framebuffer = gl.create_framebuffer().map_err(InteropError::OpenGl)?;
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(draw_framebuffer));
        gl.framebuffer_texture_2d(
            glow::DRAW_FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(gl_texture),
            0,
        );

        let read_framebuffer = std::num::NonZeroU32::new(source_fbo).map(glow::NativeFramebuffer);
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, read_framebuffer);

        let (w, h) = (size.width as i32, size.height as i32);
        gl.blit_framebuffer(0, 0, w, h, 0, h, w, 0, glow::COLOR_BUFFER_BIT, glow::NEAREST);
        gl.flush();

        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
        gl.delete_framebuffer(draw_framebuffer);
    }
    Ok(())
}
