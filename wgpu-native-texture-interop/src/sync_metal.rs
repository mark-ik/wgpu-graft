//! Metal `MTLSharedEvent` synchronizer for cross-API texture handoff.
//!
//! Pairs a wgpu Metal consumer with a producer that signals an
//! `MTLSharedEvent` on its command queue after rendering. This is the Apple
//! analog of the D3D12 fence path.
//!
//! **This synchronizer is precautionary.** IOSurface coherence is implicit
//! on Apple silicon (unified memory) and via IOSurface locks on Intel Macs;
//! no GPU sync is required for correctness on the IOSurface-backed import
//! path. The synchronizer exists for the case where empirical coherence
//! ever fails on a discrete-GPU configuration, and as the API anchor for a
//! future GPU-side wait once `wgpu-hal::metal::Queue` exposes its raw
//! `MTLCommandQueue`.
//!
//! Today the wait is **CPU-side**:
//! [`waitUntilSignaledValue:timeoutMS:`](https://developer.apple.com/documentation/metal/mtlsharedevent/waituntilsignaledvalue:timeoutms:)
//! blocks the calling thread inside `producer_complete` until the producer
//! signals the latest advanced value. A GPU-side wait via
//! `encodeWaitForEvent:value:` requires access to the wgpu Metal command
//! queue, which `wgpu-hal` 29 does not currently expose.

use std::sync::atomic::{AtomicU64, Ordering};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{MTLDevice, MTLSharedEvent, MTLSharedEventHandle};

use crate::{
    HostWgpuContext, ImportedTexture, InteropBackend, InteropError, InteropSynchronizer,
    NativeFrame, SyncMechanism,
};

/// Synchronizer that uses an `MTLSharedEvent` to gate the consumer on
/// producer rendering completion via a CPU-side wait.
pub struct MetalSharedEventSynchronizer {
    shared_event: Retained<ProtocolObject<dyn MTLSharedEvent>>,
    next_value: AtomicU64,
    timeout_ms: u64,
}

unsafe impl Send for MetalSharedEventSynchronizer {}
unsafe impl Sync for MetalSharedEventSynchronizer {}

impl MetalSharedEventSynchronizer {
    /// Default wait timeout (5 seconds). Picks up dropped signals as an
    /// explicit error rather than hanging the consumer indefinitely.
    pub const DEFAULT_TIMEOUT_MS: u64 = 5_000;

    /// Create a new shared event on the host's wgpu Metal device.
    ///
    /// Returns [`InteropError::BackendMismatch`] if `host.backend` is not
    /// [`InteropBackend::Metal`].
    pub fn new(host: &HostWgpuContext) -> Result<Self, InteropError> {
        Self::with_timeout(host, Self::DEFAULT_TIMEOUT_MS)
    }

    /// Create a new shared event with a custom CPU-wait timeout in
    /// milliseconds.
    pub fn with_timeout(host: &HostWgpuContext, timeout_ms: u64) -> Result<Self, InteropError> {
        if host.backend != InteropBackend::Metal {
            return Err(InteropError::BackendMismatch {
                expected: "Metal",
                actual: "non-Metal",
            });
        }

        let shared_event = unsafe {
            let hal_device = host.device.as_hal::<wgpu::wgc::api::Metal>().ok_or(
                InteropError::BackendMismatch {
                    expected: "Metal",
                    actual: "non-Metal",
                },
            )?;
            let device = hal_device.raw_device().clone();
            drop(hal_device);

            device
                .newSharedEvent()
                .ok_or_else(|| InteropError::Metal("newSharedEvent returned nil".into()))?
        };

        Ok(Self {
            shared_event,
            next_value: AtomicU64::new(0),
            timeout_ms,
        })
    }

    /// Returns a `MTLSharedEventHandle` for cross-process producers.
    /// Same-process producers can hold a reference to
    /// [`shared_event`](Self::shared_event) directly.
    pub fn new_shared_event_handle(&self) -> Retained<MTLSharedEventHandle> {
        unsafe { self.shared_event.newSharedEventHandle() }
    }

    /// The underlying `MTLSharedEvent`. Same-process producers can hold a
    /// reference to this and call `encodeSignalEvent:value:` on their own
    /// command buffers.
    pub fn shared_event(&self) -> &ProtocolObject<dyn MTLSharedEvent> {
        &self.shared_event
    }

    /// Increment the event counter and return the new value. The producer
    /// signals the shared event at this value after its render work is
    /// recorded.
    pub fn advance(&self) -> u64 {
        self.next_value.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The current event value (highest value returned by
    /// [`advance`](Self::advance), or 0 if `advance` has not been called).
    pub fn current_value(&self) -> u64 {
        self.next_value.load(Ordering::SeqCst)
    }
}

impl InteropSynchronizer for MetalSharedEventSynchronizer {
    fn producer_complete(
        &self,
        _frame: &NativeFrame,
        mechanism: SyncMechanism,
    ) -> Result<(), InteropError> {
        match mechanism {
            SyncMechanism::ExplicitFence => {
                let target = self.next_value.load(Ordering::SeqCst);
                if target == 0 {
                    // No advance() yet; treat as no-op.
                    return Ok(());
                }
                let signaled = unsafe {
                    self.shared_event
                        .waitUntilSignaledValue_timeoutMS(target, self.timeout_ms)
                };
                if !signaled {
                    return Err(InteropError::Metal(format!(
                        "MTLSharedEvent wait timed out at value {} after {}ms",
                        target, self.timeout_ms
                    )));
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
            | SyncMechanism::ExplicitFence => Ok(()),
            other => Err(InteropError::UnsupportedSynchronization(other)),
        }
    }
}
