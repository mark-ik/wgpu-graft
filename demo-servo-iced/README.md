# demo-servo-iced

Servo embedded in an [iced](https://github.com/iced-rs/iced) 0.14 application with a URL bar and full input forwarding.

## What it demonstrates

- Servo rendering offscreen via CPU readback (`read_full_frame()`)
- Frames displayed as an `iced::widget::image` using `allocate()` for flicker-free GPU upload
- Elm-architecture message passing for URL bar input, navigation, and Servo tick scheduling
- Mouse, scroll, and keyboard events forwarded to Servo via iced subscriptions
- Winit-based key mapping via shared `keyutils` module

## Frame upload strategy

Iced's wgpu renderer uploads images larger than 2MB asynchronously. A typical 1280x750 RGBA frame is ~3.8MB, so naively creating a new `Handle` each tick causes flicker (the async upload never finishes before the handle changes). This demo solves it by calling `iced::widget::image::allocate()` to pre-allocate the GPU texture, ensuring it's ready for the next frame.

## Usage

```bash
cargo run -p demo-servo-iced                         # built-in animated fixture
cargo run -p demo-servo-iced -- https://example.com  # load a URL
cargo run -p demo-servo-iced -- servo.org            # auto-prefixes https://
```

## Platform notes

- Uses CPU readback on all platforms.
- **Windows**: requires ANGLE DLLs next to the executable (see [workspace README](../README.md#prerequisites)).

## License

MIT OR Apache-2.0
