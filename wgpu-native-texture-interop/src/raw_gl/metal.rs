//! IOSurface-to-wgpu import via Metal (Apple platforms).
//!
//! Imports an IOSurface into a `wgpu::Texture` by:
//! 1. Creating a Metal texture backed by the IOSurface
//! 2. Wrapping it as a wgpu HAL texture
//! 3. Normalizing from BGRA8Unorm (Metal native) to RGBA8Unorm with Y-flip

use dpi::PhysicalSize;
use foreign_types_shared::ForeignType;
use objc2::runtime::NSObject;
use objc2::{msg_send, rc::Retained};
use objc2_io_surface::IOSurfaceRef;
use objc2_metal::{MTLPixelFormat, MTLTextureDescriptor, MTLTextureType, MTLTextureUsage};

use crate::InteropError;

use super::texture_normalizer::ImportedTextureNormalizer;

/// Caches the normalization pipeline for repeated IOSurface imports.
///
/// Create once and reuse across frames to avoid repeated shader compilation.
pub struct MetalImporter {
    normalizer: ImportedTextureNormalizer,
}

impl MetalImporter {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            normalizer: ImportedTextureNormalizer::new(device),
        }
    }

    /// Import an IOSurface into a normalized `wgpu::Texture`.
    ///
    /// The returned texture is Rgba8Unorm with top-left origin.
    pub fn import(
        &self,
        iosurface: &IOSurfaceRef,
        size: PhysicalSize<u32>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<wgpu::Texture, InteropError> {
        let metal_texture = self.create_metal_texture(device, iosurface, size)?;
        let hal_texture = self.wrap_as_wgpu_hal(device, metal_texture, size)?;
        Ok(self.normalizer.normalize(device, queue, &hal_texture, size))
    }

    fn create_metal_texture(
        &self,
        wgpu_device: &wgpu::Device,
        iosurface: &IOSurfaceRef,
        size: PhysicalSize<u32>,
    ) -> Result<Retained<NSObject>, InteropError> {
        unsafe {
            let metal_device = wgpu_device.as_hal::<wgpu::wgc::api::Metal>().ok_or(
                InteropError::BackendMismatch {
                    expected: "Metal",
                    actual: "non-Metal",
                },
            )?;

            let device_raw = metal_device.raw_device().clone();

            let descriptor = MTLTextureDescriptor::new();
            descriptor.setDepth(1);
            descriptor.setSampleCount(1);
            descriptor.setWidth(size.width as usize);
            descriptor.setHeight(size.height as usize);
            descriptor.setMipmapLevelCount(1);
            descriptor.setUsage(MTLTextureUsage::ShaderRead);
            descriptor.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
            descriptor.setTextureType(MTLTextureType::Type2D);

            let texture: Option<Retained<NSObject>> = msg_send![
                &*(device_raw.as_ptr() as *mut objc2::runtime::NSObject),
                newTextureWithDescriptor:&*descriptor,
                iosurface:iosurface,
                plane: 0usize
            ];

            texture.ok_or_else(|| {
                InteropError::Metal("failed to create Metal texture from IOSurface".to_string())
            })
        }
    }

    fn wrap_as_wgpu_hal(
        &self,
        wgpu_device: &wgpu::Device,
        metal_texture: Retained<NSObject>,
        size: PhysicalSize<u32>,
    ) -> Result<wgpu::Texture, InteropError> {
        unsafe {
            let ptr: *mut objc2_foundation::NSObject = Retained::into_raw(metal_texture);
            let metal_texture = metal::Texture::from_ptr(ptr as *mut _);

            let hal_texture = wgpu::hal::metal::Device::texture_from_raw(
                metal_texture,
                wgpu::TextureFormat::Bgra8Unorm,
                metal::MTLTextureType::D2,
                0,
                0,
                wgpu::hal::CopyExtent {
                    width: size.width,
                    height: size.height,
                    depth: 0,
                },
            );

            Ok(
                wgpu_device.create_texture_from_hal::<wgpu::wgc::api::Metal>(
                    hal_texture,
                    &wgpu::TextureDescriptor {
                        label: Some("iosurface-metal-import"),
                        size: wgpu::Extent3d {
                            width: size.width,
                            height: size.height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Bgra8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    },
                ),
            )
        }
    }
}
