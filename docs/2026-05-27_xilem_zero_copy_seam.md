# Xilem zero-copy: the masonry_winit External-layer seam

Status and design for getting `demo-servo-xilem` from CPU readback to a
zero-copy imported-texture composite.

## Done so far

- **grafting + adapter** build against wgpu 28 and 29 via cargo features
  (default `wgpu-29`); verified on Windows for Vulkan/DX12/GL. The version
  dimension is handled by the library, so the host is never patched for wgpu.
- **Phase 1**: `demo-servo-xilem` now rides **masonry-main (wgpu 28)** +
  `servo-wgpu-interop-adapter` `wgpu-28`. Compiles, readback path intact.
  Drift ported: `ObjectFit::Fill`→`Stretch`, `SizedBox::expand_*`→flex,
  `task_raw` closure now takes `&mut State`.
- **Seam branch**: `wgpu-graft-external-seam` worktree at
  `c:\Users\mark_\Code\crates\xilem-graft-seam`, off `upstream/main`
  @ `4eae66d` (= the rev the demo locks). Push to `mark-ik/xilem` when it
  compiles; then point the demo's git deps at that branch.

## Why a host patch is needed at all

masonry's high-level `image` view takes a CPU `peniko::ImageData`; there is no
public path to composite an external `wgpu::Texture`. masonry-main *scaffolds*
the seam (`PaintLayerMode::External` → `VisualLayerKind::External { bounds }`,
documented as "before host integration lands") but `masonry_winit` drops those
layers. Realizing them is the seam.

## Render path (rev 4eae66d, `masonry_winit/src/event_loop_runner.rs`)

1. `redraw()` → `visual_layers`; the code collects `overlay_layers()` and
   `root_layer()`, asserting each is `VisualLayerKind::Scene` and
   `unreachable!()`-ing otherwise (lines ~672-690). **This is where External
   layers are currently dropped.**
2. `PreparedFrame::new(...)` from `root_scene` + `overlays` (line ~691).
3. `render()` (line ~708): `renderer.render_to_texture(target =
   surface.target_texture, frame)` rasterizes the Vello scenes into
   `surface.target_texture`.
4. `present_surface()` (line ~777): `surface.blitter.copy(target_view →
   surface_texture)` then `surface_texture.present()`.

## Seam design

Three parts. The masonry side is verifiable with `cargo check -p masonry_winit`
in the worktree (no Servo build needed); only the demo side pulls Servo.

### 1. masonry_core (small)
`VisualLayerKind::External { bounds }` already exists. For a single external
surface (the Servo viewport) bounds alone suffice; if multiple external
surfaces are ever needed, add an `ExternalId` to the variant and key the
registry below by it. Start id-less.

### 2. masonry_winit (the real work)
- **Collect, don't panic.** In the layer loop (~672-690), route
  `VisualLayerKind::External { bounds }` into a `Vec<Rect>` (or `Option<Rect>`
  for the single-surface case) instead of `unreachable!()`. Root stays Scene.
- **External texture registry.** Add an `Option<wgpu::Texture>` (id-keyed map
  later) to the per-window render state, plus a public setter on
  `MasonryState` (e.g. `set_external_texture(Option<wgpu::Texture>)`). The app
  populates it each frame with the imported Servo texture.
- **Composite pass.** In `render()`, after `render_to_texture` and before
  `present_surface`: if there is a registered external texture and an External
  bounds, run a small textured-quad render pass onto `surface.target_view`,
  `set_viewport`/`set_scissor` to `bounds` (scaled by `scale_factor`), sampling
  the external texture. Needs a tiny pipeline (a fullscreen-quad shader plus a
  bind group), modeled on `demo-servo-winit`'s `draw_fullscreen_quad` /
  `render_texture`. `wgpu::util::TextureBlitter` blits whole-surface only, so a
  sub-rect composite needs the viewport-scoped quad, not the blitter.
- Origin/format already normalized by grafting (`ImportOptions` default
  Y-flips and converts to `Rgba8Unorm`), so the composite shader is trivial.

### 3. demo (Phase 4)
- A custom "external surface" Xilem view / masonry `Widget` whose `paint`
  calls `PaintCtx::set_paint_layer_mode(PaintLayerMode::External)` to emit the
  `External { bounds }` layer for the viewport region, replacing the current
  `image` view.
- **Seam A** (Phase 2, folds in here): in `AppDriver::on_wgpu_ready(&WgpuContext)`
  grab `device`/`queue`, build `ServoWgpuInteropAdapter::new(device, queue, size)`
  on masonry's own device.
- Per frame (in `about_to_wait`, replacing `take_frame()` readback): call
  `import_current_frame_default()` → `ImportedTexture`, hand
  `imported.texture` to `masonry_state.set_external_texture(...)`, request
  redraw. The composite pass paints it at the external widget's bounds.

## Verify

- `cargo check -p masonry_winit` in the worktree after parts 1-2 (fast, no Servo).
- Push branch to `mark-ik/xilem`; switch `demo-servo-xilem` git deps from
  `linebender/xilem` to `mark-ik/xilem` + `wgpu-graft-external-seam`.
- `cargo run -p demo-servo-xilem` (pulls Servo; disk is fine) and confirm the
  page renders with no CPU readback in the frame loop.

## Upstream

Parts 1-2 finish an upstream-sanctioned scaffold, so this branch is a
candidate PR to Linebender (permissive policy). If it lands, the demo's patch
shrinks toward zero once a masonry release carries it.
