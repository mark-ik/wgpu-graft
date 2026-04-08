# Runtime Validation

This workspace ships multiple demos embedding Servo in different GUI frameworks, plus a standalone GL→wgpu demo. This document covers what to validate and how.

## Quick commands

```bash
# Core crate tests
cargo test -p wgpu-native-texture-interop

# Build checks
cargo check -p servo-wgpu-interop-adapter --features servo
cargo check -p demo-servo-winit
cargo check -p demo-servo-xilem
cargo check -p demo-servo-iced
cargo check -p demo-servo-gpui
cargo check -p demo-raw-gl

# Run demos
cargo run -p demo-servo-winit
cargo run -p demo-servo-xilem
cargo run -p demo-servo-iced
cargo run -p demo-servo-gpui
cargo run -p demo-raw-gl
```

On Windows without `nasm`, prefix with `AWS_LC_SYS_NO_ASM=1`. In PowerShell:

```powershell
$env:AWS_LC_SYS_NO_ASM=1; cargo run -p demo-servo-winit
```

## Windows: ANGLE DLLs

Servo requires ANGLE on Windows. The `mozangle` crate builds `libEGL.dll` and `libGLESv2.dll` during compilation, but they may not end up next to the executable — especially when using a custom `CARGO_TARGET_DIR`.

Find them in `target/debug/build/mozangle-*/out/` and copy to your target's `debug/` directory.

## What to validate (all Servo demos)

1. **Startup**: the demo window opens without panics.
2. **First paint**: a web page appears (not a blank or solid-color window).
3. **Animation**: the default `animated.html` fixture updates continuously, not just one frame.
4. **Resize**: the content tracks the window size without stretching or freezing.
5. **Navigation**: clicking links navigates to new pages.
6. **Scrolling**: mouse wheel scrolls long pages.
7. **Text input** (demos with URL bar): typing in the URL bar and pressing Enter navigates.
8. **Keyboard forwarding**: keyboard events reach the web page (e.g., Tab to move focus, arrow keys to scroll).
9. **Repeated navigation**: loading several URLs in sequence does not crash.

## Demo-specific notes

### demo-servo-winit

- Logs the URL, host backend, and capability matrix to stdout on startup.
- Window title shows the active backend, sync mode, and imported texture size.
- Tries GPU import first; falls back to CPU readback if GL extensions are missing.
- No URL bar — pass URLs via command line.

### demo-servo-xilem

- URL bar + Go button above the viewport.
- Frame delivery via `tokio::sync::watch` channel.

### demo-servo-iced

- URL bar above the viewport.
- Uses `image::allocate()` for flicker-free frame upload.

### demo-servo-gpui

- URL bar above the viewport with focus management.
- RGBA→BGRA conversion for GPUI's `RenderImage` format.
- Continuous rendering via `request_animation_frame()`.

### demo-raw-gl

- No Servo dependency — renders a spinning GL triangle.
- Validates the core interop layer independently.
- Should show a smoothly spinning triangle on all supported platforms.

## Fixtures

Each Servo demo includes fixtures in its `fixtures/` directory:

- `animated.html` — frame counter + CSS animations for redraw validation.
- `static.html` (winit only) — static page for orientation and color checks.

## Platform expectations

| Platform | GPU import | CPU readback | demo-raw-gl |
| --- | --- | --- | --- |
| Linux | Works on compatible drivers | Works | Works |
| macOS | Works | Works | Works |
| Windows | Blocked (Servo forces ANGLE) | Works | Works (with compatible GL drivers) |
