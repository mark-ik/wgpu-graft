# wgpu-graft

Rust workspace for embedding [Servo](https://servo.org) web content into host applications. It provides the low-level texture interop plumbing (GL/Vulkan/Metal/DX12 → wgpu) and a set of reference demos showing how to embed Servo in different GUI frameworks. Derived from a [Slint](https://slint.dev/blog/using-servo-with-slint) embedding [example](https://github.com/slint-ui/slint/tree/master/examples/servo).

If you're looking to embed a web renderer in your Rust application, start with the demo closest to your stack and adapt from there. No promises! These are generated reference implementations to see what's possible. The demos are a bit rough but should be straightforward to understand and adapt.

To be clear and upfront: I used AI for pretty much all of it, adapting the Slint folks' Servo embedding example, and I think it turned out pretty well, considering I really just wanted to see Servo in some more esoteric GUI frameworks. I don't have Linux or Mac hardware to test those, so contributions are very welcome, but I would direct them to the Slint repo linked above first and foremost!

## Crates

| Crate | Purpose |
| --- | --- |
| [`grafting`](grafting/) | Core library: imports native GPU textures (GL FBO, Vulkan image, Metal IOSurface) into host-owned `wgpu` textures. Framework-agnostic, no Servo dependency required. |
| [`servo-wgpu-interop-adapter`](servo-wgpu-interop-adapter/) | Servo-specific adapter: wraps Servo's offscreen rendering context and bridges it to the core interop crate. Provides `ServoWgpuRenderingContext` for CPU readback and `ServoWgpuInteropAdapter` for zero-copy GPU import. |

## Demos

Each demo embeds Servo in a different Rust GUI framework to show that the approach generalizes.

| Demo | Framework | Rendering path | Notes |
| --- | --- | --- | --- |
| [`demo-servo-winit`](demo-servo-winit/) | winit + wgpu (no toolkit) | zero-copy GPU import (+ CPU fallback) | Bare-minimum reference. Fullscreen quad samples the imported texture. No URL bar — pass URLs via CLI. |
| [`demo-servo-egui`](demo-servo-egui/) | [egui](https://github.com/emilk/egui)/eframe 0.34 | zero-copy GPU import | eframe forced to DX12; `register_native_texture`. URL bar. CPU readback feature-gated. |
| [`demo-servo-iced`](demo-servo-iced/) | [iced](https://github.com/iced-rs/iced) 0.15-dev | zero-copy GPU import (shared handle) | `shader` widget; its `Primitive` is `Send`, so the frame crosses as a D3D12 shared handle. wgpu 28. URL bar. |
| [`demo-servo-blitz`](demo-servo-blitz/) | [Blitz](https://github.com/DioxusLabs/blitz) (anyrender_vello → [vello](https://github.com/linebender/vello) 0.9) | zero-copy GPU import | `try_register_custom_resource` → vello `register_texture`, drawn in the scene. |
| [`demo-servo-slint`](demo-servo-slint/) | [Slint](https://slint.dev) 1.16 | zero-copy GPU import | Official `unstable-wgpu-28`: rendering notifier for the device + `Image::try_from(wgpu::Texture)`. The example this repo was forked from. |
| [`demo-servo-xilem`](demo-servo-xilem/) | [Xilem](https://github.com/linebender/xilem) 0.4 | CPU readback | Reactive UI with URL bar. masonry/peniko image display. |
| [`demo-servo-gpui`](demo-servo-gpui/) | [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) (glass-hq wgpu fork) | CPU readback | Zed's UI framework. RGBA→BGRA `RenderImage`. Zero-copy update pending. |
| [`demo-raw-gl`](demo-raw-gl/) | glutin + glow | GPU import | Standalone GL→wgpu demo (spinning triangle). No Servo dependency — proves the interop layer works independently. |

The newer demos (egui, iced, Blitz, Slint) target Windows + DX12, where the zero-copy path uses an ANGLE-D3D11 → DX12 shared texture. They forward mouse/scroll/keyboard except Slint (display-only for now). Because they pin different wgpu versions (iced/Slint on 28, the rest on 29), build them **individually** (`cargo run -p <demo>`), not with `--workspace`.

### Rendering paths

**GPU import (zero-copy):** Servo renders to a GL framebuffer, which is imported directly into a host `wgpu` texture via platform-specific interop (Vulkan external memory, Metal IOSurface, or an ANGLE-D3D11 → DX12 shared texture on Windows). The host then samples that texture in its own renderer. No CPU round-trip. Used by winit, egui, iced, Blitz, and Slint.

**Shared-handle variant:** when a framework only exposes its `wgpu` device behind a `Send`-bounded render callback (so Servo's non-`Send` GL context can't ride along), the producer exports the D3D12 shared NT handle and the consumer opens it on the framework's own device. This is how the iced demo composites inside its `shader` widget; the same seam (`grafting::import_dx12_shared_texture`) is reusable for any such framework.

**CPU readback (fallback):** Servo renders offscreen, pixels are read back to CPU via `read_full_frame()`, then uploaded to the host's image widget. Works everywhere but adds a GPU→CPU→GPU round-trip per frame. Used by the xilem and GPUI demos today; the winit demo tries GPU import first and falls back to CPU readback if the host driver/backend cannot import the frame.

## Quick start

```bash
# Core crate tests
cargo test -p grafting

# Build check (requires Servo git dependency)
cargo check -p servo-wgpu-interop-adapter --features servo

# Run a demo (build individually — demos pin different wgpu versions)
cargo run -p demo-servo-winit
cargo run -p demo-servo-egui
cargo run -p demo-servo-iced
cargo run -p demo-servo-blitz
cargo run -p demo-servo-slint
cargo run -p demo-servo-xilem
cargo run -p demo-servo-gpui
cargo run -p demo-raw-gl
```

Pass a URL to any Servo demo:

```bash
cargo run -p demo-servo-winit -- https://servo.org
cargo run -p demo-servo-iced -- https://example.com
```

## Branches

The repository is organized around Servo compatibility lines so embedders can
pick a branch without digging through commit history.

| Branch | Purpose | Servo line |
| --- | --- | --- |
| `main` | Recommended default for embedders | current Servo LTS release line |
| `latest-release` | Tracks the newest non-LTS Servo release once one exists beyond the current LTS line | newest post-LTS release line |
| `experimental` | Integration work against upstream Servo head | upstream `main` |

`main` is the branch most users should follow. `latest-release` only diverges
once Servo ships a newer stable, non-LTS release beyond the current LTS line.

## Platform support

| Platform | GPU import | CPU readback | Notes |
| --- | --- | --- | --- |
| Linux | GL FBO → Vulkan image → wgpu | Yes | Primary development target |
| macOS | IOSurface → Metal → wgpu | Yes | |
| Windows | ANGLE D3D11 → DX12 shared texture by default; `WGPU_BACKEND=vulkan` uses the ANGLE D3D11 → Vulkan path | Yes | The winit demo exercises GPU import first and falls back to CPU readback if sharing is unavailable. |

## Prerequisites

- **Rust 1.92+** (pinned in `rust-toolchain.toml`; required by wgpu 29)
- **Servo current LTS release** on `main` (resolved via Cargo dependency)
- **Windows**: ANGLE DLLs (`libEGL.dll`, `libGLESv2.dll`) must be next to the executable at runtime. They're built by `mozangle` during compilation — find them in `target/debug/build/mozangle-*/out/` and copy to `target/debug/`. If using a custom `CARGO_TARGET_DIR`, copy them there too.
- **Windows without nasm**: set `AWS_LC_SYS_NO_ASM=1` before building (Servo pulls `aws-lc-rs`).

## How to embed Servo in your own application

The demos are designed as copy-and-adapt references. The general pattern:

1. **Add dependencies**: `servo`, `servo-wgpu-interop-adapter` (with `features = ["servo"]`), and your GUI framework.
2. **Initialize Servo**: Create a `ServoWgpuRenderingContext`, build a `Servo` instance with `ServoBuilder`, create a `WebView` with `WebViewBuilder`, and navigate to a URL.
3. **Pump the event loop**: Call `servo.spin_event_loop()` each frame to let Servo process network/layout/paint work.
4. **Get the frame**:
   - *Zero-copy (preferred):* build a `ServoWgpuInteropAdapter` on your framework's `wgpu` device and call `import_current_frame_default()` to get a `wgpu::Texture` each frame.
   - *CPU readback:* call `render_context.read_full_frame()` to get an `RgbaImage`.
5. **Display**: sample the imported texture in your renderer, or convert the image to your framework's image type.
6. **Forward input**: Convert your framework's mouse/keyboard events to Servo's `InputEvent` types and call `webview.notify_input_event()`.

See [`demo-servo-winit/src/main.rs`](demo-servo-winit/src/main.rs) for the simplest zero-copy path, [`demo-servo-iced/src/main.rs`](demo-servo-iced/src/main.rs) for the shared-handle variant, or [`demo-servo-xilem/src/main.rs`](demo-servo-xilem/src/main.rs) for CPU readback.

## Workspace patches

The `patches/` directory contains local forks and compatibility patches needed to keep Servo, wgpu 29, and the GUI demos building together:

- **`patches/glass-gpui`**: Vendored glass-hq GPUI fork with local Linux build fixes. The demo depends on published `gpui = 0.2.2`, and Cargo redirects it here so GPUI uses its wgpu-based renderer instead of the older blade/naga stack.
- **`patches/taffy-0.9`**: Vendored taffy 0.9.2 source under a 0.9.0 version declaration so GPUI's exact `=0.9.0` pin can coexist with Servo's layout dependencies.
- **`patches/serde_fmt`**: Removes an `impl From<serde_fmt::Error> for std::fmt::Error` that creates ambiguous type resolution in stylo's `ToCss` derive macro on Rust 1.92.
- **`patches/yeslogic-fontconfig-sys`** and the `glslopt` git override: Linux build compatibility fixes for the current Servo dependency stack.

The GPUI-related patches are only needed by `demo-servo-gpui`; the Servo build fixes are workspace-wide because Servo is shared by all Servo demos.

## License

[MPL-2.0](LICENSE)
