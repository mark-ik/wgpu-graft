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

    let surface = device
        .unbind_surface_from_context(&mut context)
        .map_err(|err| InteropError::Surfman(format!("{err:?}")))?
        .ok_or(InteropError::InvalidFrame("no surfman surface available"))?;

    let native_surface = device.native_surface(&surface);
    let io_surface = &native_surface.0;

    let importer = crate::raw_gl::metal::MetalImporter::new(&host.device);
    let result = importer.import(io_surface, source.size, &host.device, &host.queue);

    let rebind = device
        .bind_surface_to_context(&mut context, surface)
        .map_err(|(err, mut surface)| {
            let _ = device.destroy_surface(&mut context, &mut surface);
            InteropError::Surfman(format!("rebind after import failed: {err:?}"))
        });

    let texture = result?;
    rebind?;

    Ok(ImportedTexture {
        texture,
        format: wgpu::TextureFormat::Rgba8Unorm,
        size: frame.size(),
        origin: TextureOrigin::TopLeft,
        generation: source.generation,
        consumer_sync: SyncMechanism::None,
    })
}
