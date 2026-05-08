---
name: slint upstream sync, branch propagation, and 0.2 publish prep
description: Plan to absorb slint examples/servo upstream changes (Apr–May 2026), propagate main onto sibling branches, and stage the wgpu-native-texture-interop 0.2.0 release.
type: plan
date: 2026-05-07
---

# wgpu-graft: slint upstream sync + maintenance + update plan

## Context

`wgpu-graft` was forked from `slint-ui/slint/examples/servo` on 2026-04-08 (initial commit `67228e9`). Since that fork point, the slint upstream has shipped substantive improvements to the same Servo embedding code path. This plan absorbs the relevant upstream changes, fans them out across our branch lines, and stages a 0.2.0 release of the published interop crate. **Crates.io publishing is deferred to a follow-up session per user direction.**

## Slint upstream commits since fork point

| SHA | Date | Title | Decision |
|---|---|---|---|
| `ae6bda2` | 2026-04-13 | Use Servo v0.1.0 from crates.io | ✅ already absorbed (Cargo.lock confirms) |
| `042175d` | 2026-04-15 | servo_example: GPU rendering on Windows using Direct X | 🟢 **PORT** (P1) |
| `9645f48` | 2026-04-16 | servo example: Let Slint create the wgpu-28 instance | ❌ slint-shape; demos own wgpu already |
| `1d6c7de` | 2026-04-17 | move Vulkan logic to a dedicated module | ✅ already split per-platform |
| `e87fabd` | 2026-04-20 | Optimize adapter selection and backend synchronization | 🟢 **PORT** (P2) |
| `f1e6428` | 2026-04-20 | Update README and improve code | ⏭️ skipped (mostly slint-coupled re-exports) |
| `445200d` | 2026-04-22 | Refactor metal implementation | 🟢 **PORT** (P3) |
| `4b46b98` | 2026-04-24 | Mobile and slint code related Improvements | ⏭️ skipped (mobile UI is slint-coupled) |
| `e5ea5b4` | 2026-05-07 | PointerEvent public in language module | ❌ slint internal |

User-approved scope: **P1–P3 only**.

## Per-branch state, before any work

Branch line-up was simplified to three: `main`, `latest-release`, `experimental`.

```text
main                          canonical; wgpu 29; servo 0.1.0; webview2 fence sync; MPL-2.0
latest-release                stale; pre-MPL relicense; missing v0.1.1 bump and recent work
experimental                  stale; same shape as latest-release; predates 018eaab branch-line setup
```

The fact that `experimental` and `latest-release` are *missing* recent main commits means they're not real branch lines yet — they're stale snapshots. They were created 2026-04-08 with intent to be CI-synced (`018eaab Establish Servo branch lines and nightly experimental sync`), but no sync has actually propagated. Those branches should be hard-reset to a known main baseline as part of this pass.

Three sibling branches that existed before this pass were retired (deleted on remote and locally): `servo-0.0.6-wgpu-28` (Servo 0.0.6 + wgpu 28 maintenance line), `servo-webgl-interop` (custom WebGL interop work), `wry-webview2-texture-spike` (WebView2 / D3D11 shared-handle exploration).

## Work breakdown

### Phase 0 — verification on main (no porting yet)

Establish a green baseline before adding new code.

1. `cargo check --workspace` on Linux + Windows (CI exists; trigger or run locally).
2. `cargo test -p wgpu-native-texture-interop` (unit tests).
3. Manual run of `demo-servo-winit https://example.com` on Windows to confirm the existing CPU readback + ANGLE D3D11→Vulkan paths still work.
4. Document baseline output of the new `print_wgpu_backend` helper (added in P2) before adding it, so the before/after is visible in the next demo run.

Exit criteria: green check + green tests + a Servo demo renders.

### Phase 1 — port slint's DirectX context (P1)

**Source:** `examples/servo/src/webview/rendering_context/directx.rs` (slint `042175d` + later refinements through `e87fabd`).

**Target:** new module `wgpu-native-texture-interop/src/raw_gl/angle_dx12_shared.rs` (sibling to `angle_d3d11.rs`).

**What it does:**
1. Creates an `ID3D11Texture2D` with `D3D11_RESOURCE_MISC_SHARED | D3D11_RESOURCE_MISC_SHARED_NTHANDLE` on the *ANGLE D3D11 device* obtained via `surfman::Device::native_device().d3d11_device`.
2. Wraps it as a transient EGL pbuffer using `surfman::Device::create_surface_texture_from_texture` so Servo's GL renders into the shared texture.
3. Opens the same NT handle on the host wgpu DX12 device via `ID3D12Device::OpenSharedHandle`.
4. Wraps the DX12 resource as a `wgpu::Texture` via `wgpu_hal::dx12::Device::texture_from_raw` + `create_texture_from_hal::<Dx12>`.

**Why this is complementary, not redundant:** `raw_gl/angle_d3d11.rs` covers ANGLE→Vulkan. `raw_gl/dx12.rs` aimed for ANGLE→DX12 via `GL_EXT_memory_object_win32`, but ANGLE doesn't expose that extension, so it returns `BackendMismatch`. Slint's approach gets ANGLE→DX12 working by going through D3D11-shared-texture + EGL pbuffer wrapper.

**Differences vs slint upstream we will need to reconcile:**
- **wgpu version:** slint is on wgpu 28; wgpu-graft is on wgpu 29. The hal API for `texture_from_raw` is stable across; verify signatures.
- **glow version:** wgpu-graft uses glow 0.16/0.17; slint is on glow 0.16. Should be compatible.
- **Error type:** slint defines `DirectXTextureError` locally; wgpu-graft has a unified `InteropError` enum. Add a new variant `DirectXShared(String)` rather than a parallel error type.
- **Source FBO selection:** slint pulls the FBO from `eglGetCurrentSurface(EGL_DRAW)` after binding; wgpu-graft passes `source_fbo` explicitly. Adapt slint's blit path to take an explicit FBO so it composes with the existing `surfman_gl/windows.rs` calling convention.
- **Dispatch:** wire into `surfman_gl/windows.rs::import_current_frame` as a third path, after `angle_d3d11` Vulkan and before the GL_EXT slow path:
  ```
  ANGLE-D3D11 + host=Vulkan        → angle_d3d11.rs            (existing)
  ANGLE-D3D11 + host=DX12          → angle_dx12_shared.rs      (NEW)
  non-ANGLE Vulkan GL              → windows.rs (Vulkan opaque) (existing)
  non-ANGLE D3D12-import           → dx12.rs                    (existing, ANGLE-incompatible)
  ```

**Capability matrix:** add `angle_dx12_shared: Status` reporting `Supported` on Windows + DX12 host; update `lib.rs` reporting + tests.

### Phase 2 — LUID-matched adapter selection + backend observability (P2)

**Source:** slint `e87fabd` — `pick_synchronized_adapter` + `print_wgpu_backend`.

**Target:** apply in two places:

1. **`servo-wgpu-interop-adapter`** — when constructing `ServoWgpuInteropAdapter`, the host wgpu adapter LUID should match the surfman D3D11 device's LUID on Windows. Today wgpu-graft just trusts wgpu's default adapter; on multi-GPU systems (Intel iGPU + Arc/NVIDIA dGPU) wgpu may pick the dGPU while surfman/ANGLE binds the iGPU, breaking the shared-handle path silently.
   - Add `select_adapter_matching_surfman_luid(instance, surfman_device, mode) -> wgpu::Adapter`.
   - `mode: AdapterMode = ::Vulkan | ::Dx12` typed (per slint's final-form choice in `e87fabd`).
   - Cross-platform: this is a Windows-only concern. On Linux/macOS, fall back to `request_adapter` defaults.

2. **All `demo-servo-*`** crates that construct their own `wgpu::Instance` — invoke the new helper.
   - Demos that are NOT Windows-DX-relevant (gpui/iced/xilem on CPU readback) can keep current adapter request.

**`print_wgpu_backend(adapter: &wgpu::Adapter)`** lives in `wgpu-native-texture-interop` next to capability matrix; just logs `name | backend | driver`. Used by `demo-servo-winit` on startup.

### Phase 3 — Metal cleanup (P3)

**Source:** slint `445200d` — replace panics with `MetalTextureError` enum, inline IOSurface/texture creation, simplify wgpu type imports, move shared helpers to module roots.

**Target:** `wgpu-native-texture-interop/src/raw_gl/metal.rs` (132 LOC) and `surfman_gl/metal/`.

**Concrete edits:**
- Add `InteropError::Metal(MetalError)` variant where today some code paths `unwrap()` or `expect()`.
- Inline the BGRA→RGBA normalizer call at the metal entry rather than as a separately scheduled pass when the only caller is the Apple path.
- Remove any remaining `wpgu_hal` typo'd imports (slint had this); wgpu-graft uses `wgpu_hal` so likely already correct, but grep & confirm.

This is correctness/quality, not a feature add. No CapabilityMatrix change.

### Phase 4 — propagate to sibling branches

Scope simplified mid-pass: only `main`, `latest-release`, and `experimental` are kept. `servo-0.0.6-wgpu-28`, `servo-webgl-interop`, and `wry-webview2-texture-spike` were retired (deleted on remote and locally) — see "Per-branch state" above.

In dependency order:

1. **`main`** — receives Phase 1–3.
2. **`latest-release`** — currently stale snapshot from 2026-04-08. Hard-reset to `main` once main is green; this branch only diverges when a non-LTS Servo release ships *and* we choose to track it. There isn't one today.
3. **`experimental`** — same situation; hard-reset to `main`. The "experimental tracks Servo head" intent only matters once we wire the CI sync workflow to actually run; that's separate work, not in this pass.

For each branch: green `cargo check`, push, observe CI.

### Phase 5 — release prep (no publishing)

Update `CHANGELOG.md` `[Unreleased]` section under the existing 0.2.0 staging:

```markdown
### Added — `wgpu-native-texture-interop` 0.2.0

- `raw_gl::angle_dx12_shared`: ANGLE D3D11 → wgpu DX12 zero-copy import path
  via shared NT-handle on a transient EGL pbuffer surface. Adapted from slint
  examples/servo (#11089). Closes the gap where `raw_gl::dx12` could not
  service ANGLE-Servo (which lacks `GL_EXT_memory_object_win32`).
- `select_adapter_matching_surfman_luid`: Windows multi-GPU adapter selection
  helper that matches wgpu's adapter LUID to surfman's underlying D3D11
  device. Adapted from slint examples/servo (#11439).
- `print_wgpu_backend`: backend observability helper.
```

Bump `wgpu-native-texture-interop/Cargo.toml` from `0.2.0` to a release-ready `0.2.0` (already there) and tag the commit `wgpu-native-texture-interop-v0.2.0` *without* publishing. `servo-wgpu-interop-adapter` stays at `0.1.0` (unpublished) — first publish pairs naturally with this 0.2.0 release.

**Publish gate (next session):**
- `cargo publish --dry-run -p wgpu-native-texture-interop`
- `cargo publish --dry-run -p servo-wgpu-interop-adapter`
- Confirm dry-runs clean, then real publish.

## Verification matrix

| Check | Target | When |
|---|---|---|
| `cargo check --workspace` | linux + windows | end of every phase |
| `cargo test -p wgpu-native-texture-interop` | linux + windows | end of every phase |
| `demo-servo-winit https://servo.org` with `WGPU_BACKEND=dx12` | windows | end of Phase 1 |
| `demo-servo-winit` on dual-GPU host | windows + multi-GPU | end of Phase 2 |
| `demo-servo-winit https://servo.org` with no env override | macOS | end of Phase 3 |
| Per-branch `cargo check` after merge/reset | main + latest-release + experimental | end of Phase 4 |
| `cargo publish --dry-run` (no publish) | both crates | end of Phase 5 |

## Risks / open questions

1. **wgpu-hal DX12 API stability between 28 and 29.** `texture_from_raw` and `create_texture_from_hal::<Dx12>` were verified against the wgpu 29 docs during Phase 1 — same shape as slint's wgpu 28 source.
2. **EGL pbuffer wrapper lifetime in the new path.** Slint's directx.rs creates the surface texture *each frame*. wgpu-graft now caches the size-dependent state via `AngleDx12SharedCache` on `SurfmanFrameProducer`, matching slint's `D3D11SizeDependentState` semantics.
3. **CI workflow `experimental-servo-sync.yml`** exists but doesn't appear to have run successfully. Out of scope for this plan, but flag for follow-up — if `experimental` is supposed to track Servo HEAD nightly, the workflow needs auditing separately.

## Out of scope (explicit)

- Slint mobile UI changes (`4b46b98`).
- The "let slint create the wgpu instance" architectural change (`9645f48`) — wgpu-graft's split between core interop crate and demos already places the wgpu-instance ownership in the demo, which is a different shape than slint's.
- Any `cargo publish` actions.
