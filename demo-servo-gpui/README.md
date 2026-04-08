# demo-servo-gpui

Servo embedded in a [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) 0.2 application (Zed's UI framework) with a URL bar and full input forwarding.

## What it demonstrates

- Servo rendering offscreen via CPU readback (`read_full_frame()`)
- RGBA→BGRA pixel conversion for GPUI's `RenderImage` format
- Continuous rendering via `request_animation_frame()` in the `Render` implementation
- URL bar with focus management via `FocusHandle`
- Mouse, scroll, and keyboard events forwarded to Servo via GPUI's element event handlers
- Custom key mapping (GPUI uses its own key types, not winit)

## Usage

```bash
cargo run -p demo-servo-gpui                         # built-in animated fixture
cargo run -p demo-servo-gpui -- https://example.com  # load a URL
cargo run -p demo-servo-gpui -- servo.org            # auto-prefixes https://
```

## GPUI-specific notes

- GPUI is pre-1.0 and does not use winit, so this demo has its own key mapping in `keyutils.rs` rather than sharing the winit-based one used by the other demos.
- GPUI's `Pixels` type has a crate-private field, so all pixel math uses `f32::from(px)` rather than direct field access.
- The workspace includes two patches under `patches/` to resolve dependency conflicts specific to this demo (taffy version alignment with gpui, and a serde_fmt ambiguous impl fix for Rust 1.92). These patches are not needed by the other demos.

## Platform notes

- Uses CPU readback on all platforms.
- **Windows**: requires ANGLE DLLs next to the executable (see [workspace README](../README.md#prerequisites)).

## License

MIT OR Apache-2.0
