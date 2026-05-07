# demo-servo-xilem

Servo embedded in a [Xilem](https://github.com/linebender/xilem) 0.4 reactive UI with a URL bar and full input forwarding.

## What it demonstrates

- Servo rendering offscreen via CPU readback (`read_full_frame()`)
- Frames converted to `peniko::ImageData` and displayed via Xilem's image widget
- Frame delivery through a `tokio::sync::watch` channel with Xilem `task_raw` for reactive updates
- URL bar with navigation on Enter
- Mouse, scroll, and keyboard events in the viewport forwarded to Servo

## Usage

```bash
cargo run -p demo-servo-xilem                         # built-in animated fixture
cargo run -p demo-servo-xilem -- https://example.com  # load a URL
```

## Architecture

Xilem uses the masonry layout engine and winit for windowing. Servo events are delivered through a watch channel so the Xilem view tree rebuilds reactively when new frames arrive. Key mapping reuses winit's key types via a shared `keyutils` module.

## Platform notes

- Uses CPU readback on all platforms. GPU import is not integrated here — this demo focuses on showing the Xilem integration pattern.
- **Windows**: requires ANGLE DLLs next to the executable (see [workspace README](../README.md#prerequisites)).

## License

[MPL-2.0](../LICENSE)
