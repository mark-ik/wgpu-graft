# wgpu-graft

Rust workspace for grafting an external GPU producer's texture onto host-owned
`wgpu` textures, plus reference demos that embed [Servo](https://servo.org) web
content into different Rust GUI frameworks. It provides the low-level texture
interop plumbing (GL FBO / Vulkan image / Metal IOSurface / DX12 shared texture →
`wgpu`) in the framework-agnostic `grafting` crate, a Servo-specific adapter on
top of it, and eight demos showing the embedding pattern across stacks.

Derived from the
[Slint Servo embedding example](https://github.com/slint-ui/slint/tree/master/examples/servo)
(see [the Slint blog post](https://slint.dev/blog/using-servo-with-slint)).

The demos are copy-and-adapt reference implementations. If you want to embed a
web renderer in a Rust app, start from the demo closest to your stack and adapt.
They were largely AI-generated while exploring what Servo embedding looks like in
less common frameworks. Contributions, especially Linux/macOS validation, are
welcome.

**Made with AI**

## Crates

| Crate | Version | Purpose |
| --- | --- | --- |
| [`grafting`](grafting/) | 0.3.0 | Core library. Imports native GPU textures (GL FBO via surfman, Vulkan image, Metal IOSurface, DX12/D3D11 shared NT handle) into host-owned `wgpu` textures. Framework-agnostic; the core import paths require no Servo dependency. Carries wgpu at two majors behind features (`wgpu-29` default, `wgpu-28`) so it can build against whichever wgpu your host already uses. |
| [`servo-wgpu-interop-adapter`](servo-wgpu-interop-adapter/) | 0.1.0 | Servo-specific adapter built on `grafting`. Wraps Servo's offscreen rendering context and bridges it to the interop layer. Provides `ServoWgpuRenderingContext` (CPU readback) and `ServoWgpuInteropAdapter` (zero-copy GPU import). Servo support is behind the `servo` feature. |
| [`demo-support`](demo-support/) | 0.1.0 | Shared helpers for the demos. On Windows it forces `mozangle` to build the ANGLE runtime DLLs (`libEGL.dll` / `libGLESv2.dll`) so each demo's `build.rs` can copy them next to the executable. |

## Demos

Each demo embeds Servo in a different Rust GUI framework to show that the
approach generalizes. All Servo demos use the
`servo` git dependency on `branch = "release/v0.2"`.

| Demo | Framework | Host wgpu | Rendering path | Notes |
| --- | --- | --- | --- | --- |
| [`demo-servo-winit`](demo-servo-winit/) | winit + wgpu (no toolkit) | 29 | zero-copy GPU import (+ CPU fallback) | Bare-minimum reference. Fullscreen quad samples the imported texture. No URL bar; pass URLs via CLI. |
| [`demo-servo-egui`](demo-servo-egui/) | egui/eframe 0.34 | 29 | zero-copy GPU import | eframe forced to DX12; `register_native_texture`. URL bar. CPU readback feature-gated. |
| [`demo-servo-iced`](demo-servo-iced/) | iced 0.15-dev (git rev `4255f613`) | 28 | zero-copy GPU import (shared handle) | `shader` widget; its `Primitive` is `Send`, so the frame crosses as a D3D12 shared handle. URL bar. |
| [`demo-servo-blitz`](demo-servo-blitz/) | Blitz (anyrender_vello → vello 0.9) | 29 | zero-copy GPU import | `try_register_custom_resource` → vello `register_texture`, drawn in the scene. |
| [`demo-servo-slint`](demo-servo-slint/) | Slint 1.16 | 28 | zero-copy GPU import | Official `unstable-wgpu-28`: rendering notifier for the device + `Image::try_from(wgpu::Texture)`. The example this repo was forked from. Display-only (no input forwarding). |
| [`demo-servo-bevy`](demo-servo-bevy/) | Bevy 0.19.0-rc.2 | 29 | zero-copy GPU import (shared handle) | Render world runs on its own thread, so it uses the shared-handle seam like iced; imports then copies into a Bevy-owned `GpuImage` (resize-safe). Display-only for now. |
| [`demo-servo-xilem`](demo-servo-xilem/) | Xilem 0.4 | n/a | CPU readback | Reactive UI with URL bar. masonry/peniko image display. |
| [`demo-servo-gpui`](demo-servo-gpui/) | GPUI (glass-hq fork via `gpui_wgpu`) | 29 | CPU readback | Zed's UI framework. RGBA→BGRA `RenderImage`. Zero-copy update pending. |
| [`demo-raw-gl`](demo-raw-gl/) | glutin + glow | n/a | GPU import | Standalone GL→wgpu demo (spinning triangle). No Servo dependency; proves the interop layer works independently. |

Because the demos pin different wgpu versions (iced/Slint on 28, the rest on 29),
build them individually with `cargo run -p <demo>`, not with `--workspace`. The
newer demos (egui, iced, Blitz, Slint, Bevy) target Windows + DX12, where the
zero-copy path uses an ANGLE-D3D11 → DX12 shared texture.

### Rendering paths

**GPU import (zero-copy):** Servo renders to a GL framebuffer, which is imported
directly into a host `wgpu` texture via platform-specific interop (Vulkan
external memory, Metal IOSurface, or an ANGLE-D3D11 → DX12 shared texture on
Windows). The host samples that texture in its own renderer. No CPU round-trip.
Used by winit, egui, Blitz, and Slint (in-process import).

**Shared-handle variant:** when a framework only exposes its `wgpu` device behind
a `Send`-bounded render callback or on a separate render thread (so Servo's
non-`Send` GL context cannot ride along), the producer exports the D3D12 shared
NT handle and the consumer opens it on the framework's own device via
`grafting::import_dx12_shared_texture`. This is how iced (its `shader` widget) and
Bevy (its render world) composite the Servo frame.

**CPU readback (fallback):** Servo renders offscreen, pixels are read back to CPU
via `read_full_frame()`, then uploaded to the host's image widget. Works
everywhere but adds a GPU→CPU→GPU round-trip per frame. Used by the xilem and
GPUI demos today; the winit demo tries GPU import first and falls back to CPU
readback if the host driver/backend cannot import the frame.

## Quick start

```bash
# Core crate tests
cargo test -p grafting

# Build check of the Servo adapter (requires the Servo git dependency)
cargo check -p servo-wgpu-interop-adapter --features servo

# Run a demo (build individually; demos pin different wgpu versions)
cargo run -p demo-servo-winit
cargo run -p demo-servo-egui
cargo run -p demo-servo-iced
cargo run -p demo-servo-blitz
cargo run -p demo-servo-slint
cargo run -p demo-servo-bevy
cargo run -p demo-servo-xilem
cargo run -p demo-servo-gpui
cargo run -p demo-raw-gl
```

Pass a URL to any Servo demo:

```bash
cargo run -p demo-servo-winit -- https://servo.org
cargo run -p demo-servo-iced -- https://example.com
```

## Prerequisites

- **Rust 1.95.0** (pinned in `rust-toolchain.toml`). The floor is 1.95 because
  Bevy 0.19.0-rc.2 requires it; wgpu 29 alone needs 1.92, and the iced/Slint
  (wgpu 28) demos need 1.88.
- **Servo `release/v0.3`** for the Servo demos (resolved via the `servo` git
  dependency; no local Servo checkout needed).
- **Windows**: ANGLE DLLs (`libEGL.dll`, `libGLESv2.dll`) must be next to the
  executable at runtime. They are built by `mozangle` during compilation (via the
  `demo-support` crate's `build_dlls` feature) and copied next to the binary by
  each demo's `build.rs`. If using a custom `CARGO_TARGET_DIR`, ensure they land
  there too.
- **Windows without nasm**: set `AWS_LC_SYS_NO_ASM=1` before building (Servo pulls
  `aws-lc-rs`).
- **Windows clang toolchain (Servo demos)**: build from a **VS Developer Command
  Prompt** so MSVC `cl.exe` is on PATH (`aws-lc-sys` and SpiderMonkey need it). If a
  standalone LLVM is installed alongside Visual Studio's bundled clang, pin both the
  compiler and bindgen to one clang version, or mozangle's bindgen step fails with
  `use of undeclared identifier '__builtin_ia32_*'` in `mmintrin.h` (clang 21 dropped
  the legacy MMX builtins that the clang-19-era ANGLE headers still reference, and
  `LIBCLANG_PATH` defaulting to the newer clang while the compiler uses the older one
  is the split):

  ```cmd
  set CC=clang-cl
  set CXX=clang-cl
  set LIBCLANG_PATH=C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\Llvm\x64\bin
  ```

  If `cargo` can't fetch Servo's large git repo via libgit2, also set
  `CARGO_NET_GIT_FETCH_WITH_CLI=true`.

## Platform support

| Platform | GPU import | CPU readback | Notes |
| --- | --- | --- | --- |
| Linux | GL FBO → Vulkan external memory FD → wgpu | Yes | All demos build and run (verified on Fedora 44 / Mesa-RADV / Vulkan). |
| macOS | IOSurface → Metal → wgpu (BGRA→RGBA normalize) | Yes | |
| Windows | ANGLE D3D11 → DX12 shared texture by default; `WGPU_BACKEND=vulkan` uses the ANGLE D3D11 → Vulkan path. A non-ANGLE `GL_EXT_memory_object_win32` path also exists where supported. | Yes | The winit demo tries GPU import first and falls back to CPU readback if sharing is unavailable. LUID-matched adapter selection keeps the shared handle single-GPU on multi-GPU machines. |

## How to embed Servo in your own application

The demos are designed as copy-and-adapt references. The general pattern:

1. **Add dependencies**: `servo`, `servo-wgpu-interop-adapter` (with
   `features = ["servo"]`), and your GUI framework. Match your host's wgpu major
   by selecting `wgpu-29` (default) or `wgpu-28` on the adapter/`grafting`.
2. **Initialize Servo**: create a `ServoWgpuRenderingContext`, build a `Servo`
   instance with `ServoBuilder`, create a `WebView` with `WebViewBuilder`, and
   navigate to a URL.
3. **Pump the event loop**: call `servo.spin_event_loop()` each frame.
4. **Get the frame**:
   - *Zero-copy (preferred):* build a `ServoWgpuInteropAdapter` on your
     framework's `wgpu` device and call `import_current_frame_default()` to get a
     `wgpu::Texture` each frame. For frameworks that own their device on a
     separate thread, export a D3D12 shared handle with
     `current_dx12_shared_texture()` and open it on the consumer side with
     `grafting::import_dx12_shared_texture`.
   - *CPU readback:* call `render_context.read_full_frame()` to get an
     `RgbaImage`.
5. **Display**: sample the imported texture in your renderer, or convert the image
   to your framework's image type.
6. **Forward input**: convert your framework's mouse/keyboard events to Servo's
   `InputEvent` types and call `webview.notify_input_event()`.

See [`demo-servo-winit/src/main.rs`](demo-servo-winit/src/main.rs) for the
simplest zero-copy path, [`demo-servo-iced/src/main.rs`](demo-servo-iced/src/main.rs)
or [`demo-servo-bevy/src/main.rs`](demo-servo-bevy/src/main.rs) for the
shared-handle variant, or [`demo-servo-xilem/src/main.rs`](demo-servo-xilem/src/main.rs)
for CPU readback.

## Workspace patches

`patches/glass-gpui` is its own cargo workspace (the vendored glass-hq/gpui fork)
and is excluded from this workspace via `[workspace] exclude`. The other patches
are wired through `[patch.crates-io]` in the root `Cargo.toml`:

- **`patches/glass-gpui`**: vendored glass-hq/gpui fork (a wgpu-based,
  Zed-tracking gpui fork that renders through `gpui_wgpu` on wgpu 29 instead of
  the older blade/naga stack) with two local Linux build fixes from its "extract
  platform crates" refactor: `ashpd` bumped 0.12.1 → 0.13, and a re-added
  `LayerShellNotSupportedError`. Cargo redirects `gpui` here. Needed only by
  `demo-servo-gpui`.
- **`patches/taffy-0.9`**: taffy 0.9.2 source vendored under a `0.9.0` version
  declaration so GPUI's exact `=0.9.0` pin is satisfied with newer code while
  Servo's layout still resolves its `^0.10` request to crates.io.
- **`patches/serde_fmt`**: removes an `impl From<serde_fmt::Error> for
  std::fmt::Error` that creates ambiguous `From` resolution in stylo's `ToCss`
  derive macro on the pinned toolchain.
- **`patches/yeslogic-fontconfig-sys`**: emits both the `extern "C"` and the
  `dlopen` runtime-loaded forms unconditionally so feature unification between
  `servo-fonts` and xilem's `fontique` cannot break either consumer.
- **`glslopt` git override** (to `jamienicol/glslopt-rs`): build fix for a C11
  `once_flag` collision on glibc 2.34+ until webrender bumps glslopt.

The GPUI and taffy patches matter only to `demo-servo-gpui`; the Servo build
fixes are workspace-wide because Servo is shared by all Servo demos.

## Branches

The repository is organized around Servo compatibility lines. CI workflows in
`.github/workflows/` keep these in sync.

| Branch | Purpose | Servo line |
| --- | --- | --- |
| `main` | Recommended default for embedders | current Servo release line (`release/v0.2`) |
| `latest-release` | Tracks the newest non-LTS Servo release once one exists beyond the current line | newest post-LTS release line |
| `experimental` | Integration work against upstream Servo head | upstream `main` |

## Relationship to sibling repos

`wgpu-graft` is the origin of a set of standalone wgpu interop libraries (graft /
weld / scry): `wgpu-scry` was extracted from it and keeps its Slint-derived
`native_frame` structure, and `wgpu-weld` follows the same import pattern for the
CEF / Chromium engine. It was renamed from `wgpu-gui-bridge` (2026-05-05); "graft" carries the
surgical sense of joining an external GPU resource onto a wgpu host. The bare
`graft` name was taken on crates.io, hence the `wgpu-graft` workspace name and the
gerund crate name `grafting`. The project is complementary to WebRender's wgpu
backend work: GL interop is useful while Servo's GL path persists, and would
simplify to same-device texture sharing once a production wgpu backend lands.

## Repository layout

```
grafting/                     core interop crate (no Servo dependency required)
servo-wgpu-interop-adapter/   Servo-specific adapter on top of grafting
demo-support/                 shared demo helpers (ANGLE DLL build glue on Windows)
demo-servo-*/                 one demo per GUI framework (winit/egui/iced/blitz/
                              slint/bevy/xilem/gpui)
demo-raw-gl/                  standalone GL→wgpu demo, no Servo
patches/                      local forks and compatibility patches
docs/                         design docs, plans, and testing notes
scripts/smoke-demo.ps1        demo smoke-test script
CHANGELOG.md                  release notes
```

## License

[MPL-2.0](LICENSE). See also [NOTICE](NOTICE).
