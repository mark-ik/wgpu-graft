# wgpu-native-texture-interop

Import native GPU textures (GL framebuffers, Vulkan images, Metal IOSurfaces) into host-owned `wgpu` textures. This is the core library of the [servo-wgpu-interop](../) workspace.

This crate is **framework-agnostic** — it has no Servo dependency and can be used by any application that needs to import externally-produced GPU content into wgpu.

## What it does

A "producer" (Servo, a GL renderer, a video decoder, etc.) renders into a native GPU resource. This crate imports that resource into a `wgpu::Texture` owned by the host application, enabling zero-copy compositing across API boundaries.

## Platform support

| Platform | Import path | Status |
| --- | --- | --- |
| Linux / Android | GL FBO → Vulkan external memory → wgpu | Implemented |
| macOS / iOS | IOSurface → Metal texture → wgpu | Implemented |
| Windows (Vulkan) | GL FBO → Vulkan image (NT handle) → wgpu | Implemented |
| Windows (DX12) | GL FBO → DX12 shared texture → wgpu | Builds, runtime deferred |

## Key types

- `HostWgpuContext` — wraps the host's `wgpu::Device` and `wgpu::Queue`
- `NativeFrame` — platform-specific frame produced by the offscreen renderer
- `ImportedTexture` — the result of importing a `NativeFrame` into wgpu
- `ImportOptions` — controls import behavior (format, usage flags)
- `CapabilityMatrix` — runtime query of what the current platform/driver supports
- `InteropBackend` — detected backend (Vulkan, Metal, DX12)
- `FrameProducer` / `TextureImporter` — traits for the produce/import pipeline
- `WgpuTextureImporter` — default `TextureImporter` implementation
- `InteropSynchronizer` — trait for cross-API synchronization policies

## Modules

- `raw_gl` — surfman-independent GL import functions. Use `RawGlFrameProducer` for any GL application without bringing surfman as a dependency (set `default-features = false`).
- `surfman_gl` — surfman-backed frame producer (enabled by default via the `surfman` feature).

## Usage

For Servo embedding, pair this crate with [`servo-wgpu-interop-adapter`](../servo-wgpu-interop-adapter/) which handles Servo-specific setup. For standalone GL import, see [`demo-raw-gl`](../demo-raw-gl/) which uses the `raw_gl` module directly.

```toml
[dependencies]
# Full (with surfman support):
wgpu-native-texture-interop = "0.1"

# Minimal (raw GL only, no surfman):
wgpu-native-texture-interop = { version = "0.1", default-features = false }
```

## License

MIT OR Apache-2.0
