---
name: bevy + gpui zero-copy demos, plus deferred cleanup
description: Plan + verified findings for the two remaining zero-copy demos (Bevy, gpui) and the deferred cleanup, after six frameworks shipped 2026-06-01/02.
type: plan
date: 2026-06-02
---

# Bevy + gpui zero-copy demos (remaining) + deferred cleanup

## Status — six frameworks shipped (confirmed by Mark)

All zero-copy Servo → host-framework, confirmed on the Windows multi-GPU laptop:

| Demo | Host wgpu | Import path | Notes |
|---|---|---|---|
| `demo-servo-winit` | 29 | in-process GL import | reference; resize fixed (sole driver = `webview.resize`) |
| `demo-servo-egui` (new) | 29 | in-process import | eframe forced DX12; `register_native_texture` |
| `demo-servo-iced` (new) | 28 (git rev) | **shared-handle** | `shader` widget Primitive is `Send`; iced 0.15-dev |
| `demo-servo-blitz` (new) | 29 | in-process import | anyrender_vello `try_register_custom_resource` → vello `register_texture` |
| `demo-servo-slint` (new) | 28 | in-process import | slint 1.16 `unstable-wgpu-28`; notifier + `Image::try_from` |

Plus the grafting core work: the multi-GPU **flicker fix** (`select_adapter_matching_surfman_luid` via `new_for_device`), single canonical normalize-flip, the **shared-handle seam** (`grafting::import_dx12_shared_texture` + `SurfmanFrameProducer::export_dx12_shared_texture` + `ServoWgpuRenderingContext::current_dx12_shared_texture`), and normalizer `COPY_SRC`.

Screenshots: each demo's `screenshots/<name>.png` (Mark saving them; READMEs already reference them).

## Verified version landscape (2026-06-02)

- **Bevy:** 0.18.1 stable = wgpu **27**; 0.19.0-rc.2 = wgpu **29.0.3**; `main` (0.19.0-dev) = wgpu **29.0.3**. → Use **0.19.0-rc.2** (no new grafting version; published crates.io release). No newer wgpu on dev.
- **gpui:** zed-industries upstream gpui = **blade** renderer. The **glass-hq fork** (vendored at `patches/glass-gpui/`, itself Mark's `mark-ik/wgpu-gui-bridge`-tracked copy) replaces blade with **wgpu 29 + naga 29** and adds a `gpui_wgpu` crate. So the gpui demo composites on **wgpu 29**, NOT blade. (Confirm glass-hq's exact upstream URL + whether it has newer commits in the fresh session — couldn't pin the repo URL this pass.)
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

Existing demo = CPU readback (`CapturingRenderingContext` → BGRA `RenderImage`, driven by `request_animation_frame`; Servo `!Send` on the main thread). Update to zero-copy:

- glass-gpui is wgpu 29 and runs the renderer on the main thread (gpui is single-threaded UI) → **in-process import** (like Blitz/Slint), no shared handle.
- The **vendored patch** (authorized) goes to `patches/glass-gpui`: add a way for gpui to present an external `wgpu::Texture` as an image/surface (gpui's image path today is the CPU `RenderImage`). Likely a new `gpui_wgpu` image-source or a primitive that samples a provided `wgpu::Texture`. Import Servo's frame onto gpui's wgpu device and feed it through that.
- Confirm how to access gpui's `wgpu::Device`/`Queue` (via `gpui_wgpu` — there should be a renderer/context accessor) so `ServoWgpuInteropAdapter::new` can LUID-match surfman to it.

## Deferred cleanup (do alongside / after)

1. **iced cpu-readback feature** — re-add the feature-gated CPU readback path to `demo-servo-iced` (dropped to ship zero-copy first).
2. **Dead `ImportingRenderingContext`** in `servo-wgpu-interop-adapter` — the present-hook path is unused now (demos use the plain rendering context + post-paint import); remove it.
3. **Minimal sync set review** — with LUID-match as the real flicker fix, re-check whether `present()`'s `PreserveBuffer::Yes` + `glFinish` and the normalizer copy are all still required, or if the set can shrink. Verify on winit + egui after any change.
4. **Screenshots into READMEs** — Mark saving `screenshots/<name>.png` per demo; main README references them.
5. **Main README** — updated this pass with the six demos + the wgpu-per-framework table + build-individually note.
