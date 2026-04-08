# demo-servo-winit

Minimal Servo embedding using winit + wgpu with no GUI toolkit. This is the primary reference demo for the interop layer.

## What it demonstrates

- The host owns the `wgpu::Device`, `wgpu::Queue`, and presentation surface
- Servo renders offscreen through `servo-wgpu-interop-adapter`
- GPU texture import (zero-copy) is attempted first; if the driver lacks the required GL extensions, the demo falls back to CPU readback
- Mouse, scroll, and keyboard events are forwarded to Servo for full page interactivity (clickable links, scrolling, text input)

This demo has no URL bar UI. Pass URLs via the command line; the current URL is shown in the window title. For demos with a URL bar, see the [xilem](../demo-servo-xilem/), [iced](../demo-servo-iced/), or [gpui](../demo-servo-gpui/) demos.

## Usage

```bash
cargo run -p demo-servo-winit                                  # built-in animated fixture
cargo run -p demo-servo-winit -- https://example.com           # load a URL
cargo run -p demo-servo-winit -- servo.org                     # auto-prefixes https://
cargo run -p demo-servo-winit -- demo-servo-winit/fixtures/static.html  # local file
```

## Fixtures

- `fixtures/animated.html` — continuously animating page for validating redraw scheduling and repeated frame import.
- `fixtures/static.html` — static page for checking orientation, text sharpness, and color correctness.

## Runtime diagnostics

On startup, the demo logs the URL, host backend, and capability matrix to stdout. The window title updates to show the active backend, sync mode, and imported texture size.

## Platform notes

- **Linux / macOS**: GPU import path works on compatible drivers. Falls back to CPU readback if GL extensions are missing.
- **Windows**: CPU readback only. Servo forces ANGLE (D3D-backed), so GPU import is not available until Servo supports native Vulkan/DX12 rendering.
- **Windows without nasm**: set `AWS_LC_SYS_NO_ASM=1` before building.

## License

MIT OR Apache-2.0
