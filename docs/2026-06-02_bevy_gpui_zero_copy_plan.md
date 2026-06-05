---
name: bevy + gpui zero-copy demos, plus deferred cleanup
description: Plan + verified findings for the two remaining zero-copy demos (Bevy, gpui) and the deferred cleanup, after six frameworks shipped 2026-06-01/02.
type: plan
date: 2026-06-02
---

# Bevy + gpui zero-copy demos (remaining) + deferred cleanup

## Status — seven frameworks shipped (confirmed by Mark)

All zero-copy Servo → host-framework, confirmed on the Windows multi-GPU laptop:

| Demo | Host wgpu | Import path | Notes |
|---|---|---|---|
| `demo-servo-winit` | 29 | in-process GL import | reference; resize fixed (sole driver = `webview.resize`) |
| `demo-servo-egui` (new) | 29 | in-process import | eframe forced DX12; `register_native_texture` |
| `demo-servo-iced` (new) | 28 (git rev) | **shared-handle** | `shader` widget Primitive is `Send`; iced 0.15-dev |
| `demo-servo-blitz` (new) | 29 | in-process import | anyrender_vello `try_register_custom_resource` → vello `register_texture` |
| `demo-servo-slint` (new) | 28 | in-process import | slint 1.16 `unstable-wgpu-28`; notifier + `Image::try_from` |
| `demo-servo-bevy` (new) | 29 | **shared-handle** | bevy 0.19.0-rc.2; render world on its own thread; import then `copy_texture_to_texture` into a Bevy-owned `GpuImage` (resize-safe); toolchain bumped to 1.95 |

gpui is the eighth and only remaining framework. It is **not** shipped: zero-copy needs a from-scratch render path in the engine (see the gpui section below). The other seven are committed on `main`.

Plus the grafting core work: the multi-GPU **flicker fix** (`select_adapter_matching_surfman_luid` via `new_for_device`), single canonical normalize-flip, the **shared-handle seam** (`grafting::import_dx12_shared_texture` + `SurfmanFrameProducer::export_dx12_shared_texture` + `ServoWgpuRenderingContext::current_dx12_shared_texture`), and normalizer `COPY_SRC`.

Screenshots: each demo's `screenshots/<name>.png` (Mark saving them; READMEs already reference them).

## Verified version landscape (2026-06-02)

- **Bevy:** 0.18.1 stable = wgpu **27**; 0.19.0-rc.2 = wgpu **29.0.3**; `main` (0.19.0-dev) = wgpu **29.0.3**. → Use **0.19.0-rc.2** (no new grafting version; published crates.io release). No newer wgpu on dev.
- **gpui:** the gpui demo composites on **wgpu 29** via the `gpui_wgpu` crate. Upstream is now verified (2026-06-02): `zed-industries/zed` `main` itself carries the multi-crate split (`gpui`, `gpui_wgpu`, `gpui_linux`, `gpui_macos`, `gpui_windows`, `gpui_web`, `gpui_platform`) and a wgpu 29 backend. The fork is **`Glass-HQ/gpui`** `main` (pushed 2026-05-16), which tracks zed's structure and lags it slightly. The vendored copy at `patches/glass-gpui/` matches both on the surface path. (The earlier "zed = blade only" note was stale; zed mainline has `gpui_wgpu` as of this check.)
- grafting carries wgpu **28 + 29**. iced/slint use 28; winit/egui/blitz/bevy/gpui use 29.
- Note: mixed wgpu versions in one workspace can't share a single `cargo build --workspace`; build demos individually.

## Bevy demo plan (`demo-servo-bevy`)

Bevy 0.19.0-rc.2, wgpu 29. The render world runs on a separate thread (pipelined rendering), so Servo (`!Send`, main world) and `RenderDevice` (render world) are separated → use the **shared-handle seam** (the same reason iced needs it).

Verified API (bevy_render 0.19.0-rc.2):
- `GpuImage { texture, texture_view, sampler, texture_descriptor: TextureDescriptor<Option<&'static str>, &'static [TextureFormat]>, texture_view_descriptor: Option<...>, had_data: bool }` (`src/texture/gpu_image.rs`).
- `RenderDevice::wgpu_device() -> &wgpu::Device` (`src/renderer/render_device.rs:259`); `RenderQueue` derefs to `wgpu::Queue`.
- `RenderAssets<GpuImage>::insert(id: impl Into<AssetId<Image>>, GpuImage)` (`src/render_asset.rs:224`).

Approach:
1. Main world: throwaway HighPerformance-DX12 wgpu device → `new_for_device` anchors surfman to that GPU; force Bevy to DX12+HighPerformance via `WgpuSettings` (in `RenderPlugin`). Servo as a `NonSend` resource.
2. Main-world system: paint Servo → `current_dx12_shared_texture()` → `SharedFrame { handle: u64, w, h }` resource.
3. Extract `SharedFrame` into the render world (`ExtractSchedule`).
4. Render-world system: `import_dx12_shared_texture(&frame, &HostWgpuContext::new(device.wgpu_device().clone(), (**queue).clone()))` → build `GpuImage` → `RenderAssets::<GpuImage>::insert(handle_id, gpu_image)`.
5. Main world: a fullscreen `Sprite`/UI node on a `Handle<Image>` placeholder (size/format only, `RENDER_WORLD` usage). 2D camera.

### Verified Bevy 0.19.0-rc.2 API (read from source 2026-06-02)

- `bevy::render::settings`: `WgpuSettings { backends: Option<Backends>, power_preference, device_label, features, limits, .. }` (Default); `RenderCreation::Automatic(WgpuSettings)`; `RenderPlugin { render_creation: RenderCreation, .. }`. Force DX12+HP: `DefaultPlugins.set(RenderPlugin { render_creation: RenderCreation::Automatic(WgpuSettings { backends: Some(Backends::DX12), power_preference: PowerPreference::HighPerformance, ..default() }), ..default() })`.
- `bevy::render::{RenderApp, Render, RenderSet, ExtractSchedule, Extract}` (lib.rs). `RenderApp` is the sub-app label: `app.sub_app_mut(RenderApp)`. `Extract<'w,'s, P>` is the system param to read main-world data in an `ExtractSchedule` system.
- `RenderDevice::wgpu_device() -> &wgpu::Device` (renderer/render_device.rs:259).
- `RenderQueue(pub Arc<WgpuWrapper<Queue>>)` with `Deref`/`DerefMut` (renderer/mod.rs:124) → get the wgpu queue via `(**render_queue.0).clone()` (Arc→WgpuWrapper→Queue; `wgpu::Queue: Clone`).
- `GpuImage { texture, texture_view, sampler, texture_descriptor: wgpu_types::TextureDescriptor<Option<&'static str>, &'static [TextureFormat]>, texture_view_descriptor: Option<...>, had_data: bool }`. Build: `texture.create_view(&Default::default())` for the view; sampler from `Res<DefaultImageSampler>` (`(***default_sampler).clone()`); `texture_descriptor` = a `TextureDescriptor { label: None, size, mip_level_count:1, sample_count:1, dimension: D2, format: Rgba8Unorm, usage: TEXTURE_BINDING|COPY_DST, view_formats: &[] }`.
- `RenderAssets::<GpuImage>::insert(id: impl Into<AssetId<Image>>, GpuImage)` (render_asset.rs:224). Key it by the placeholder handle's `AssetId`.
- `bevy_image::Image::new_uninit(size: Extent3d, dimension: TextureDimension, format: TextureFormat, asset_usage: RenderAssetUsages)` for the placeholder; use `RenderAssetUsages::RENDER_WORLD`. Sprite: `Sprite::from_image(handle)`; 2D camera: `Camera2d` (prelude).

### Wiring (to finalize against the build)

- Main world: `WgpuSettings` DX12; throwaway HP-DX12 device → `new_for_device` anchors surfman; Servo as `NonSendMut`/`NonSend` resource; `Assets<Image>` placeholder (`new_uninit`, RENDER_WORLD) → `Handle<Image>` on a `Sprite` + `Camera2d`. Store the handle in a `Resource`.
- Main-world `Update` system (`NonSend`): paint Servo → `current_dx12_shared_texture()` → write a `SharedFrame { handle:u64, w,h }` resource.
- `app.sub_app_mut(RenderApp)`: add an `ExtractSchedule` system using `Extract<Res<SharedFrame>>` + `Extract<Res<ServoImageHandle>>` to copy them into the render world; add the inject system to `Render` in `RenderSet::Prepare` (after `RenderSet::PrepareAssets`) that imports via `grafting::import_dx12_shared_texture` and `RenderAssets::<GpuImage>::insert`.
- Open: confirm exact `RenderSet` variant ordering (PrepareAssets → Prepare); whether to cache the imported texture per-size to avoid per-frame `OpenSharedHandle`; placeholder image format must match the imported texture (`Rgba8Unorm`, top-left from the default normalize path).

Recreate `demo-servo-bevy/` with the Cargo.toml below (scaffold + Cargo.toml committed as WIP head-start).

```toml
bevy = { version = "0.19.0-rc.2", default-features = false, features = [
    "bevy_winit", "bevy_core_pipeline", "bevy_sprite", "bevy_window", "bevy_asset", "x11" ] }
grafting = { path = "../grafting" }                 # default wgpu-29
servo-wgpu-interop-adapter = { path = "...", features = ["servo"] }   # default wgpu-29
# + servo (git release/v0.2), demo-support, euclid, url, winit, wgpu 29, pollster, rustls
```

## gpui demo plan (`demo-servo-gpui` — update)

Existing demo = CPU readback (`CapturingRenderingContext` → BGRA `RenderImage`, driven by `request_animation_frame`; Servo `!Send` on the main thread). gpui's UI renderer is single-threaded, so zero-copy can use **in-process import** (like Blitz/Slint), not the shared handle.

### Upstream verification (2026-06-02, both repos read at `main`)

Checked `zed-industries/zed` (`main`, pushed 2026-06-02) and `Glass-HQ/gpui` (`main`, pushed 2026-05-16). Findings are identical on the surface path; the vendored copy is current. There is **no** existing way to render an external `wgpu::Texture` into a gpui scene:

1. The surface fragment shader is **YUV-video-only**: `fs_surface` samples `t_y` + `t_cb_cr` (NV12 planes), not RGBA. zed `shaders.wgsl:1352`, Glass-HQ `:1323`.
2. The whole surface element is **macOS-gated**: `SurfaceSource` has only `#[cfg(target_os="macos")] Surface(CVPixelBuffer)`; `surface()`, `window.paint_surface`, and `PaintSurface` (one macOS field `image_buffer: CVPixelBuffer`) are all `#[cfg(target_os="macos")]`.
3. The wgpu **surfaces draw is a no-op stub**: `PrimitiveBatch::Surfaces` in `gpui_wgpu/src/wgpu_renderer.rs` returns `true` with the comment "Surfaces are macOS-only for video playback / Not implemented for Linux/wgpu". zed `:1303`, Glass-HQ `:1257`. The pipeline + bind-group-layout + `SurfaceParams` uniform exist (built around the YUV shader), but nothing draws.
4. **No alternative GPU entry point.** `ImageSource` is `Resource | Render | Image | Custom`, all resolving to a CPU `RenderImage`; `canvas()` is a CPU paint-instruction callback. Code search for `register_native_texture` / "external texture" is empty.
5. **No work in flight.** Zero open PRs for a wgpu surface/external-texture path; the last 30 `gpui_wgpu` commits are text/atlas/device-recovery/backend-selection only.

Two bonuses found while verifying:
- `WgpuContext` (`gpui_wgpu/src/wgpu_context.rs:9-13`) already exposes `pub device: Arc<wgpu::Device>` and `pub queue: Arc<wgpu::Queue>`. The field is public; only an app-facing accessor chain (window → renderer → context) is missing.
- `CompositorGpuHint { device_id: u32 }` exists in the same file. It may be a cleaner way to make gpui pick the surfman/ANGLE-matched GPU than the throwaway-DX12 LUID-anchor used for iced/Bevy. Probe it before falling back to the anchor.

### Patch design (from-scratch RGBA external-texture path)

The vendored patch (authorized) lives in `patches/glass-gpui`. It is the largest of the eight because it adds a render path the engine does not have. Keep it upstreamable: implementing the wgpu surfaces draw is a gap zed itself flags.

Five seams to cut, in this order:

1. **gpui core, renderer-agnostic handle.** gpui core has no wgpu dep, so the texture cannot be a `wgpu::Texture` there. Carry it as `Arc<dyn Any + Send + Sync>`. Add a non-macOS field to `PaintSurface` (`crates/gpui/src/scene.rs:715`) or a sibling primitive holding `{ bounds, content_mask, texture: Arc<dyn Any + Send + Sync> }`. Extending `PaintSurface` reuses the existing `Primitive::Surface` / `PrimitiveBatch::Surfaces` batching, which is the smaller change.
2. **paint method.** Add a non-macOS `window.paint_external_texture(bounds, Arc<dyn Any + Send + Sync>)` (mirror `paint_surface` at `crates/gpui/src/window.rs:4336`) that inserts the primitive.
3. **surface element, Windows variant.** `crates/gpui/src/elements/surface.rs`: add a cross-platform `SurfaceSource::Texture(Arc<dyn Any + Send + Sync>)`, ungate `surface()` for Windows, and add a `Surface::paint` arm routing it to `paint_external_texture`.
4. **gpui_wgpu, RGBA draw.** `crates/gpui_wgpu/src/shaders.wgsl`: add `fs_surface_rgba` that samples a single RGBA `texture_2d<f32>` (reuse `vs_surface` + `SurfaceParams` unchanged). Add a second pipeline (`surfaces_rgba`) using it. Implement `PrimitiveBatch::Surfaces`: downcast the handle to `wgpu::Texture`, make a bind group (texture view + sampler + `SurfaceParams` from bounds/content_mask), draw the quad.
5. **device accessor.** Expose gpui's `WgpuContext.device`/`.queue` to app code through a window/renderer accessor so the demo can `ServoWgpuInteropAdapter::new_for_device(gpui_device)` (LUID-match) and import each frame in-process.

### Demo wiring (`demo-servo-gpui`)

- On startup, reach gpui's wgpu device via the new accessor; build the Servo adapter with `new_for_device` (or set `CompositorGpuHint` if that path works) so surfman/ANGLE anchors to gpui's GPU.
- Each frame (`request_animation_frame`, main thread): paint Servo, `import_current_frame_default()` onto gpui's device to get a `wgpu::Texture`, wrap it `Arc::new(texture) as Arc<dyn Any + Send + Sync>`, and render `surface(SurfaceSource::Texture(arc))` sized to the view. Keep the URL bar + input forwarding from the current demo.
- Format/orientation: the default normalize path is `Rgba8Unorm`, top-left, so `fs_surface_rgba` samples straight (no V-flip, unlike the bottom-left shared-handle export used by Bevy).

### Cost + risk

Multi-file change across gpui core (scene/window/surface) and gpui_wgpu (shader/pipeline/draw/accessor), plus a multi-minute glass-gpui rebuild per iteration. Run it as its own session so a half-finished engine patch never lands on the seven shipped demos. The seven are committed; gpui starts from a clean tree.

## Deferred cleanup (do alongside / after)

1. **iced cpu-readback feature** — re-add the feature-gated CPU readback path to `demo-servo-iced` (dropped to ship zero-copy first).
2. **Dead `ImportingRenderingContext`** in `servo-wgpu-interop-adapter` — the present-hook path is unused now (demos use the plain rendering context + post-paint import); remove it.
3. **Minimal sync set review** — with LUID-match as the real flicker fix, re-check whether `present()`'s `PreserveBuffer::Yes` + `glFinish` and the normalizer copy are all still required, or if the set can shrink. Verify on winit + egui after any change.
4. **Screenshots into READMEs** — Mark saving `screenshots/<name>.png` per demo; main README references them.
5. **Main README** — updated this pass with the six demos + the wgpu-per-framework table + build-individually note.
