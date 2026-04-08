# demo-raw-gl

Standalone GL→wgpu texture import demo. A glutin+glow GL context renders a spinning triangle to an offscreen FBO, which is imported into a host-owned `wgpu` texture and composited to screen.

**No Servo dependency.** This demo proves the core `wgpu-native-texture-interop` layer works independently of Servo and surfman.

## What it demonstrates

- Creating an offscreen GL framebuffer with glow
- Importing the GL FBO into a wgpu texture via `wgpu-native-texture-interop` (using `default-features = false` to skip surfman)
- Presenting the imported texture through a wgpu fullscreen-quad render pipeline
- Continuous animation to validate repeated frame import

## Usage

```bash
cargo run -p demo-raw-gl
```

## Platform notes

- **Linux**: GL FBO → Vulkan external memory → wgpu. Requires a Vulkan-capable driver with `GL_EXT_memory_object_fd`.
- **macOS**: IOSurface-backed GL FBO → Metal texture → wgpu.
- **Windows**: GL FBO → Vulkan external memory (NT handle) → wgpu. Requires `GL_EXT_memory_object_win32`.

## License

MIT OR Apache-2.0
