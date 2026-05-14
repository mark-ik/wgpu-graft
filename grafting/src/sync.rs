use crate::{ImportedTexture, InteropError, NativeFrame};

/// Describes how the producer signals that a frame is ready and how the
/// consumer signals that it has finished reading.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SyncMechanism {
    /// No synchronization is needed (e.g. single-threaded, or the producer
    /// and consumer share a submission queue).
    None,
    /// The producer calls `glFlush()` before handing the frame over. This is
    /// the most common mechanism for GL→wgpu interop and the default used by
    /// [`ImplicitOnlySynchronizer`].
    ImplicitGlFlush,
    /// An explicit Vulkan/Metal external semaphore is signalled by the
    /// producer. Not yet handled by any built-in synchronizer.
    ExplicitExternalSemaphore,
    /// An explicit CPU-side fence is used. Not yet handled by any built-in
    /// synchronizer.
    ExplicitFence,
}

/// Hook points called by [`WgpuTextureImporter`](crate::WgpuTextureImporter)
/// around each frame import.
///
/// Implement this trait to add custom fence/semaphore logic. Two built-in
/// implementations are provided: [`NoopSynchronizer`] and
/// [`ImplicitOnlySynchronizer`].
pub trait InteropSynchronizer {
    /// Called after the frame is acquired from the producer, before import.
    /// Use this to wait on any producer-side signal.
    fn producer_complete(
        &self,
        frame: &NativeFrame,
        mechanism: SyncMechanism,
    ) -> Result<(), InteropError>;
    /// Called after the texture has been imported and is ready for the
    /// consumer. Use this to signal any consumer-side fence or semaphore.
    fn consumer_ready(
        &self,
        texture: &ImportedTexture,
        mechanism: SyncMechanism,
    ) -> Result<(), InteropError>;
}

/// A synchronizer that does nothing. Suitable when the caller manages all
/// synchronization externally (e.g. via a shared queue or explicit barriers).
#[derive(Default)]
pub struct NoopSynchronizer;

impl InteropSynchronizer for NoopSynchronizer {
    fn producer_complete(
        &self,
        _frame: &NativeFrame,
        _mechanism: SyncMechanism,
    ) -> Result<(), InteropError> {
        Ok(())
    }

    fn consumer_ready(
        &self,
        _texture: &ImportedTexture,
        _mechanism: SyncMechanism,
    ) -> Result<(), InteropError> {
        Ok(())
    }
}

/// A synchronizer that accepts [`SyncMechanism::None`] and
/// [`SyncMechanism::ImplicitGlFlush`] and rejects any explicit semaphore or
/// fence mechanism with [`InteropError::UnsupportedSynchronization`].
///
/// This is the default synchronizer used by
/// [`WgpuTextureImporter::new`](crate::WgpuTextureImporter::new). It is
/// correct for the GL→Vulkan/Metal paths implemented in this crate, which rely
/// on `glFlush()` having been called by the producer before the frame is
/// handed over.
#[derive(Default)]
pub struct ImplicitOnlySynchronizer;

impl InteropSynchronizer for ImplicitOnlySynchronizer {
    fn producer_complete(
        &self,
        _frame: &NativeFrame,
        mechanism: SyncMechanism,
    ) -> Result<(), InteropError> {
        Self::validate(mechanism)
    }

    fn consumer_ready(
        &self,
        _texture: &ImportedTexture,
        mechanism: SyncMechanism,
    ) -> Result<(), InteropError> {
        Self::validate(mechanism)
    }
}

impl ImplicitOnlySynchronizer {
    pub(crate) fn validate(mechanism: SyncMechanism) -> Result<(), InteropError> {
        match mechanism {
            SyncMechanism::None | SyncMechanism::ImplicitGlFlush => Ok(()),
            other => Err(InteropError::UnsupportedSynchronization(other)),
        }
    }
}
