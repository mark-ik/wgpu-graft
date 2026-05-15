//! Direct D3D12 shared-handle → `wgpu::Texture` import path.
//!
//! For producers that have an `ID3D12Resource` (or D3D11 resource with shared
//! NT handle) and want it imported into wgpu's DX12 backend zero-copy. The
//! producer creates the resource with `D3D12_HEAP_FLAG_SHARED` and exports an
//! NT handle via `IDXGIResource1::CreateSharedHandle`; the importer opens its
//! own reference via `ID3D12Device::OpenSharedHandle`.

use crate::{Dx12SharedTexture, HostWgpuContext, InteropBackend, InteropError};

pub fn import_dx12_shared_texture(
    frame: &Dx12SharedTexture,
    host: &HostWgpuContext,
) -> Result<wgpu::Texture, InteropError> {
    if host.backend != InteropBackend::Dx12 {
        return Err(InteropError::BackendMismatch {
            expected: "Dx12",
            actual: "non-Dx12",
        });
    }

    let texture = unsafe {
        let hal_device = host.device.as_hal::<wgpu::wgc::api::Dx12>().ok_or(
            InteropError::BackendMismatch {
                expected: "Dx12",
                actual: "non-Dx12",
            },
        )?;

        let d3d_device = hal_device.raw_device().clone();
        let mut resource: Option<windows::Win32::Graphics::Direct3D12::ID3D12Resource> = None;
        d3d_device
            .OpenSharedHandle(
                windows::Win32::Foundation::HANDLE(frame.handle as *mut std::ffi::c_void),
                &mut resource,
            )
            .map_err(|e| InteropError::Dx12(e.to_string()))?;
        let resource = resource
            .ok_or_else(|| InteropError::Dx12("OpenSharedHandle returned null".into()))?;

        let hal_texture = wgpu_hal::dx12::Device::texture_from_raw(
            resource,
            frame.format,
            wgpu::TextureDimension::D2,
            wgpu::Extent3d {
                width: frame.size.width,
                height: frame.size.height,
                depth_or_array_layers: 1,
            },
            1, // mip_level_count
            1, // sample_count
        );

        host.device.create_texture_from_hal::<wgpu::wgc::api::Dx12>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: Some("dx12-shared-texture-import"),
                size: wgpu::Extent3d {
                    width: frame.size.width,
                    height: frame.size.height,
                    depth_or_array_layers: 1,
                },
                format: frame.format,
                dimension: wgpu::TextureDimension::D2,
                mip_level_count: 1,
                sample_count: 1,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            },
        )
    };

    Ok(texture)
}
