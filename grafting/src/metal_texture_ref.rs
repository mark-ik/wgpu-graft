//! Direct `MTLTexture` → `wgpu::Texture` import path for Metal producers.
//!
//! Unlike [`crate::raw_gl::metal`], which imports a GL framebuffer through
//! IOSurface, this path wraps a raw `MTLTexture` pointer directly. The
//! producer retains ownership of the underlying texture — the importer takes
//! a +1 retain count and hands it to wgpu via `texture_from_raw`.

use foreign_types_shared::ForeignType;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;

use crate::{HostWgpuContext, InteropBackend, InteropError, MetalTextureRef};

pub fn import_metal_texture_ref(
    frame: &MetalTextureRef,
    host: &HostWgpuContext,
) -> Result<wgpu::Texture, InteropError> {
    if frame.raw_metal_texture.is_null() {
        return Err(InteropError::InvalidFrame("raw_metal_texture is null"));
    }
    if host.backend != InteropBackend::Metal {
        return Err(InteropError::BackendMismatch {
            expected: "Metal",
            actual: "non-Metal",
        });
    }

    let texture = unsafe {
        // Retain the caller's MTLTexture so that wgpu can take ownership
        // of the reference we hand it without invalidating the caller's copy.
        let obj_ptr = frame.raw_metal_texture as *mut AnyObject;
        let retained = Retained::retain(obj_ptr)
            .ok_or_else(|| InteropError::Metal("failed to retain Metal texture".into()))?;
        let raw_ptr = Retained::into_raw(retained) as *mut _;
        let metal_texture = metal::Texture::from_ptr(raw_ptr);

        let hal_texture = wgpu::hal::metal::Device::texture_from_raw(
            metal_texture,
            frame.format,
            metal::MTLTextureType::D2,
            0, // array_layers
            0, // mip_levels
            wgpu::hal::CopyExtent {
                width: frame.size.width,
                height: frame.size.height,
                depth: 0,
            },
        );

        host.device
            .create_texture_from_hal::<wgpu::wgc::api::Metal>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("metal-texture-ref-import"),
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
