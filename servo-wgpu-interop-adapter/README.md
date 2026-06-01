# servo-wgpu-interop-adapter

Servo-specific offscreen rendering adapter built on [`grafting`](../grafting/).

This crate bridges Servo's rendering context to the host application. It provides two things:

1. **`ServoWgpuRenderingContext`** — an offscreen `RenderingContext` that Servo renders into. Supports CPU readback via `read_full_frame()` (returns an `image::RgbaImage` of the current page).
2. **`ServoWgpuInteropAdapter`** — zero-copy GPU import path that imports Servo's GL framebuffer directly into a host `wgpu::Texture` via the core interop crate.

## Which path to use

- **CPU readback** (`ServoWgpuRenderingContext::read_full_frame()`): Works on all platforms. Simpler to integrate — just display the returned image in your framework's image widget. Adds a GPU→CPU→GPU round-trip per frame.
- **GPU import** (`ServoWgpuInteropAdapter`): Zero-copy, but requires compatible native sharing support between Servo's GL producer and the host wgpu backend. Linux uses Vulkan external memory, Apple uses IOSurface/Metal, and Windows supports Servo's ANGLE D3D11 output through DX12 shared textures by default or an ANGLE D3D11 → Vulkan path when `WGPU_BACKEND=vulkan` is selected.

The CPU readback demos ([xilem](../demo-servo-xilem/), [iced](../demo-servo-iced/), [gpui](../demo-servo-gpui/)) use `read_full_frame()`. The [winit demo](../demo-servo-winit/) tries GPU import first and falls back to CPU readback.

## Feature flags

- **`servo`** (optional) — enables the published `servo` crate dependency and Servo trait implementations. All Servo-embedding demos enable this feature.

Without `servo`, only the surfman-level types are available (useful for testing the adapter layer without pulling in all of Servo).

## Usage

```toml
[dependencies]
servo-wgpu-interop-adapter = { version = "0.1", features = ["servo"] }
servo = "0.1.0"
```

```rust
use servo_wgpu_interop_adapter::ServoWgpuRenderingContext;

// Create the rendering context (implements Servo's RenderingContext trait)
let render_ctx = ServoWgpuRenderingContext::new(connection, adapter, surface_type);

// After Servo paints, read the frame as an RGBA image
if let Some(rgba_image) = render_ctx.read_full_frame() {
    // Display in your framework's image widget
}
```

See the demo crates for complete integration examples.

## License

[MPL-2.0](../LICENSE)
