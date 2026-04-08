//! A `FrameProducer` for raw GL applications that don't use surfman.
//!
//! This allows any application with a GL context and a framebuffer to import
//! its rendered content into a host wgpu texture through the standard
//! `FrameProducer` / `WgpuTextureImporter` pipeline.

use std::ffi::c_void;
use std::rc::Rc;
use std::sync::Arc;

use winit::dpi::PhysicalSize;

use crate::{
    FrameProducer, GlFramebufferSource, GlFramebufferSourceImpl, HostWgpuContext, ImportOptions,
    ImportedTexture, InteropError, NativeFrame, NativeFrameKind, ProducerCapabilities,
    SyncMechanism, TextureOrigin, UnsupportedReason,
};

/// A frame producer for raw GL applications (no surfman dependency).
///
/// Wraps a GL context and framebuffer ID, producing frames that can be
/// imported into wgpu textures via [`crate::WgpuTextureImporter`].
///
/// # Platform support
///
/// - **Linux/Android**: Imports via Vulkan external memory (requires Vulkan host backend)
/// - **Windows**: Imports via Vulkan external memory with NT handles (requires Vulkan host backend)
/// - **Apple**: Not supported (use `raw_gl::metal::MetalImporter` with IOSurface instead)
///
/// # Example
///
/// ```ignore
/// use std::sync::Arc;
/// use wgpu_native_texture_interop::raw_gl::producer::RawGlFrameProducer;
/// use wgpu_native_texture_interop::{WgpuTextureImporter, HostWgpuContext, ImportOptions};
/// use winit::dpi::PhysicalSize;
///
/// let host = HostWgpuContext::new(device, queue);
/// let importer = WgpuTextureImporter::new(host);
///
/// let mut producer = RawGlFrameProducer::new(
///     gl,
///     |name| std::ptr::null(), // replace with eglGetProcAddress or equivalent
///     7, // your GL FBO id
///     PhysicalSize::new(1920, 1080),
/// );
///
/// let frame = producer.acquire_frame().unwrap();
/// let imported = importer.import_frame(&frame, &ImportOptions::default()).unwrap();
/// // Use imported.texture in your wgpu render pipeline
/// ```
pub struct RawGlFrameProducer {
    gl: Arc<glow::Context>,
    proc_loader: Rc<dyn Fn(&str) -> *const c_void>,
    source_fbo: u32,
    size: PhysicalSize<u32>,
    generation: u64,
}

impl RawGlFrameProducer {
    /// Create a new raw GL frame producer.
    ///
    /// # Arguments
    ///
    /// * `gl` - A glow GL context for the producer's GL state
    /// * `proc_loader` - Function to load GL extension entry points (e.g. wrapping
    ///   `eglGetProcAddress`, `wglGetProcAddress`, or equivalent)
    /// * `source_fbo` - The GL framebuffer object ID to read from (0 for default framebuffer)
    /// * `size` - Current framebuffer dimensions
    pub fn new(
        gl: Arc<glow::Context>,
        proc_loader: impl Fn(&str) -> *const c_void + 'static,
        source_fbo: u32,
        size: PhysicalSize<u32>,
    ) -> Self {
        Self {
            gl,
            proc_loader: Rc::new(proc_loader),
            source_fbo,
            size,
            generation: 0,
        }
    }

    /// Update the source FBO to read from.
    pub fn set_source_fbo(&mut self, fbo: u32) {
        self.source_fbo = fbo;
    }

    /// Update the framebuffer dimensions (e.g. after a resize).
    pub fn set_size(&mut self, size: PhysicalSize<u32>) {
        self.size = size;
    }
}

impl FrameProducer for RawGlFrameProducer {
    fn capabilities(&self) -> ProducerCapabilities {
        ProducerCapabilities {
            supported_frames: vec![NativeFrameKind::GlFramebufferSource],
        }
    }

    fn acquire_frame(&mut self) -> Result<NativeFrame, InteropError> {
        self.generation += 1;

        Ok(NativeFrame::GlFramebufferSource(GlFramebufferSource::new(
            self.size,
            self.generation,
            SyncMechanism::None,
            Rc::new(RawGlImportImpl {
                gl: self.gl.clone(),
                proc_loader: self.proc_loader.clone(),
                source_fbo: self.source_fbo,
                size: self.size,
                generation: self.generation,
            }),
        )))
    }
}

struct RawGlImportImpl {
    gl: Arc<glow::Context>,
    proc_loader: Rc<dyn Fn(&str) -> *const c_void>,
    source_fbo: u32,
    size: PhysicalSize<u32>,
    generation: u64,
}

impl GlFramebufferSourceImpl for RawGlImportImpl {
    fn import_into(
        &self,
        _frame: &GlFramebufferSource,
        host: &HostWgpuContext,
        _options: &ImportOptions,
    ) -> Result<ImportedTexture, InteropError> {
        let texture = self.import_texture(host)?;

        Ok(ImportedTexture {
            texture,
            format: wgpu::TextureFormat::Rgba8Unorm,
            size: self.size,
            origin: TextureOrigin::TopLeft,
            generation: self.generation,
            consumer_sync: SyncMechanism::ImplicitGlFlush,
        })
    }
}

impl RawGlImportImpl {
    fn import_texture(&self, host: &HostWgpuContext) -> Result<wgpu::Texture, InteropError> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if host.backend == crate::InteropBackend::Vulkan {
            return super::linux::import_gl_framebuffer_vulkan(
                &self.gl,
                &|name| (self.proc_loader)(name),
                self.source_fbo,
                self.size,
                host,
            );
        }

        #[cfg(target_os = "windows")]
        if host.backend == crate::InteropBackend::Vulkan {
            return super::windows::import_gl_framebuffer_vulkan_win32(
                &self.gl,
                &|name| (self.proc_loader)(name),
                self.source_fbo,
                self.size,
                host,
            );
        }

        #[cfg(target_os = "windows")]
        if host.backend == crate::InteropBackend::Dx12 {
            return super::dx12::import_gl_framebuffer_dx12(
                &self.gl,
                &|name| (self.proc_loader)(name),
                self.source_fbo,
                self.size,
                host,
            );
        }

        Err(InteropError::Unsupported(
            UnsupportedReason::HostBackendMismatch,
        ))
    }
}
