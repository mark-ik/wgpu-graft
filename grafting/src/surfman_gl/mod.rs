mod adapter;
mod surfman_frame_context;

#[cfg(any(target_os = "linux", target_os = "android"))]
mod linux;

#[cfg(target_vendor = "apple")]
mod metal;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
mod windows_dx12_shared;

pub use adapter::select_adapter_matching_surfman_luid;

use std::cell::Cell;
use std::rc::Rc;

use dpi::PhysicalSize;

use crate::{
    FrameProducer, GlFramebufferSource, GlFramebufferSourceImpl, HostWgpuContext, ImportOptions,
    ImportedTexture, InteropBackend, InteropError, NativeFrame, NativeFrameKind,
    ProducerCapabilities, SyncMechanism, UnsupportedReason,
};

pub use surfman_frame_context::SurfmanFrameContext;

pub struct SurfmanFrameProducer {
    context: Rc<SurfmanFrameContext>,
    size: Rc<Cell<PhysicalSize<u32>>>,
    generation: Cell<u64>,
    #[cfg(target_os = "windows")]
    angle_dx12_shared: Rc<windows_dx12_shared::AngleDx12SharedCache>,
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
struct SurfmanGlFrameSource {
    context: Rc<SurfmanFrameContext>,
    size: PhysicalSize<u32>,
    generation: u64,
    #[cfg(target_os = "windows")]
    angle_dx12_shared: Rc<windows_dx12_shared::AngleDx12SharedCache>,
}

impl SurfmanFrameProducer {
    pub fn new(context: Rc<SurfmanFrameContext>, initial_size: PhysicalSize<u32>) -> Self {
        Self {
            context,
            size: Rc::new(Cell::new(initial_size)),
            generation: Cell::new(0),
            #[cfg(target_os = "windows")]
            angle_dx12_shared: Rc::new(windows_dx12_shared::AngleDx12SharedCache::new()),
        }
    }

    pub fn context(&self) -> Rc<SurfmanFrameContext> {
        self.context.clone()
    }

    pub fn size(&self) -> PhysicalSize<u32> {
        self.size.get()
    }

    pub fn set_size(&self, size: PhysicalSize<u32>) {
        self.size.set(size);
    }
}

impl FrameProducer for SurfmanFrameProducer {
    fn capabilities(&self) -> ProducerCapabilities {
        ProducerCapabilities {
            supported_frames: vec![NativeFrameKind::GlFramebufferSource],
        }
    }

    fn acquire_frame(&mut self) -> Result<NativeFrame, InteropError> {
        let next_generation = self.generation.get() + 1;
        self.generation.set(next_generation);

        Ok(NativeFrame::GlFramebufferSource(GlFramebufferSource::new(
            self.size.get(),
            next_generation,
            SyncMechanism::None,
            Rc::new(SurfmanGlFrameSource {
                context: self.context.clone(),
                size: self.size.get(),
                generation: next_generation,
                #[cfg(target_os = "windows")]
                angle_dx12_shared: self.angle_dx12_shared.clone(),
            }),
        )))
    }
}

impl GlFramebufferSourceImpl for SurfmanGlFrameSource {
    fn import_into(
        &self,
        _frame: &GlFramebufferSource,
        host: &HostWgpuContext,
        _options: &ImportOptions,
    ) -> Result<ImportedTexture, InteropError> {
        match host.backend {
            #[cfg(any(target_os = "linux", target_os = "android"))]
            InteropBackend::Vulkan => linux::import_current_frame(self, _frame, host, _options),

            #[cfg(target_os = "windows")]
            InteropBackend::Vulkan => windows::import_current_frame(self, _frame, host, _options),

            #[cfg(target_vendor = "apple")]
            InteropBackend::Metal => metal::import_current_frame(self, _frame, host, _options),

            #[cfg(target_os = "windows")]
            InteropBackend::Dx12 => {
                windows::import_current_frame_dx12(self, _frame, host, _options)
            }

            #[cfg(not(target_os = "windows"))]
            InteropBackend::Dx12 => Err(InteropError::Unsupported(
                UnsupportedReason::PlatformNotImplemented,
            )),
            InteropBackend::Unknown => Err(InteropError::Unsupported(
                UnsupportedReason::HostBackendUnavailable,
            )),
            _ => Err(InteropError::Unsupported(
                UnsupportedReason::HostBackendMismatch,
            )),
        }
    }
}
