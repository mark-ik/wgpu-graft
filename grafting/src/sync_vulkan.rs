//! Vulkan external semaphore synchronizer for cross-API texture handoff.
//!
//! Pairs a wgpu Vulkan consumer with a producer that signals a `VkSemaphore`
//! after rendering. Designed for the WPE DMABUF protocol on Linux, where
//! each frame carries an `OPAQUE_FD`-handle-typed semaphore fd.
//!
//! Per frame:
//!
//! 1. The producer hands the consumer a frame whose
//!    [`VulkanExternalImage::wait_semaphore_fd`] is `Some(fd)`.
//! 2. `producer_complete` imports the fd into a persistent `VkSemaphore`
//!    using `vkImportSemaphoreFdKHR` with [`vk::SemaphoreImportFlags::TEMPORARY`].
//! 3. A standalone "pure wait" `vkQueueSubmit` (wait semaphore, no command
//!    buffers) is issued on the wgpu Vulkan queue. Subsequent wgpu submits
//!    are gated on the producer's signal.
//!
//! After the wait, the temporary import is consumed and the persistent
//! semaphore returns to its prior unsignalled state, ready for the next
//! frame. The fd ownership transfers to the driver on import per the
//! Vulkan spec — the producer must not close it after handoff.

use std::sync::Mutex;

use ash::vk;

use crate::{
    HostWgpuContext, ImportedTexture, InteropBackend, InteropError, InteropSynchronizer,
    NativeFrame, SyncMechanism,
};

/// Synchronizer that uses an external `VkSemaphore` fd per frame to gate
/// consumer submits on producer rendering completion.
pub struct VulkanSemaphoreSynchronizer {
    vk_device: ash::Device,
    vk_queue: vk::Queue,
    persistent_semaphore: vk::Semaphore,
    external_semaphore_fd: ash::khr::external_semaphore_fd::Device,
    // The wait submit and import are mutually-exclusive across threads to
    // prevent concurrent imports of the temporary payload on the same
    // semaphore handle.
    submit_lock: Mutex<()>,
}

unsafe impl Send for VulkanSemaphoreSynchronizer {}
unsafe impl Sync for VulkanSemaphoreSynchronizer {}

impl VulkanSemaphoreSynchronizer {
    /// Create a synchronizer bound to the host's wgpu Vulkan device + queue.
    ///
    /// Returns [`InteropError::BackendMismatch`] if `host.backend` is not
    /// [`InteropBackend::Vulkan`].
    pub fn new(host: &HostWgpuContext) -> Result<Self, InteropError> {
        if host.backend != InteropBackend::Vulkan {
            return Err(InteropError::BackendMismatch {
                expected: "Vulkan",
                actual: "non-Vulkan",
            });
        }

        unsafe {
            let hal_device = host.device.as_hal::<wgpu::wgc::api::Vulkan>().ok_or(
                InteropError::BackendMismatch {
                    expected: "Vulkan",
                    actual: "non-Vulkan",
                },
            )?;
            let vk_device = hal_device.raw_device().clone();
            let vk_instance = hal_device.shared_instance().raw_instance().clone();
            drop(hal_device);

            let hal_queue = host.queue.as_hal::<wgpu::wgc::api::Vulkan>().ok_or(
                InteropError::BackendMismatch {
                    expected: "Vulkan",
                    actual: "non-Vulkan",
                },
            )?;
            let vk_queue = hal_queue.as_raw();
            drop(hal_queue);

            let semaphore = vk_device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(|err| {
                    InteropError::Vulkan(format!("create_semaphore: {}", err))
                })?;

            let external_semaphore_fd =
                ash::khr::external_semaphore_fd::Device::new(&vk_instance, &vk_device);

            Ok(Self {
                vk_device,
                vk_queue,
                persistent_semaphore: semaphore,
                external_semaphore_fd,
                submit_lock: Mutex::new(()),
            })
        }
    }
}

impl InteropSynchronizer for VulkanSemaphoreSynchronizer {
    fn producer_complete(
        &self,
        frame: &NativeFrame,
        mechanism: SyncMechanism,
    ) -> Result<(), InteropError> {
        match mechanism {
            SyncMechanism::ExplicitExternalSemaphore => {
                let semaphore_fd = match frame {
                    NativeFrame::VulkanExternalImage(vk_frame) => vk_frame.wait_semaphore_fd,
                    _ => {
                        return Err(InteropError::Vulkan(
                            "ExplicitExternalSemaphore requires a VulkanExternalImage frame".into(),
                        ));
                    }
                };
                let Some(fd) = semaphore_fd else {
                    return Err(InteropError::Vulkan(
                        "frame.wait_semaphore_fd is None but mechanism is ExplicitExternalSemaphore"
                            .into(),
                    ));
                };

                let _guard = self.submit_lock.lock().map_err(|_| {
                    InteropError::Vulkan("submit_lock poisoned".into())
                })?;

                unsafe {
                    self.external_semaphore_fd
                        .import_semaphore_fd(
                            &vk::ImportSemaphoreFdInfoKHR::default()
                                .semaphore(self.persistent_semaphore)
                                .flags(vk::SemaphoreImportFlags::TEMPORARY)
                                .handle_type(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD)
                                .fd(fd),
                        )
                        .map_err(|err| {
                            InteropError::Vulkan(format!("import_semaphore_fd: {}", err))
                        })?;

                    let wait_semaphores = [self.persistent_semaphore];
                    let wait_stages = [vk::PipelineStageFlags::FRAGMENT_SHADER];
                    let submit_info = vk::SubmitInfo::default()
                        .wait_semaphores(&wait_semaphores)
                        .wait_dst_stage_mask(&wait_stages);

                    self.vk_device
                        .queue_submit(self.vk_queue, &[submit_info], vk::Fence::null())
                        .map_err(|err| InteropError::Vulkan(format!("queue_submit: {}", err)))?;
                }
                Ok(())
            }
            SyncMechanism::None | SyncMechanism::ImplicitGlFlush => Ok(()),
            other => Err(InteropError::UnsupportedSynchronization(other)),
        }
    }

    fn consumer_ready(
        &self,
        _texture: &ImportedTexture,
        mechanism: SyncMechanism,
    ) -> Result<(), InteropError> {
        match mechanism {
            SyncMechanism::None
            | SyncMechanism::ImplicitGlFlush
            | SyncMechanism::ExplicitExternalSemaphore => Ok(()),
            other => Err(InteropError::UnsupportedSynchronization(other)),
        }
    }
}

impl Drop for VulkanSemaphoreSynchronizer {
    fn drop(&mut self) {
        unsafe {
            self.vk_device
                .destroy_semaphore(self.persistent_semaphore, None);
        }
    }
}
