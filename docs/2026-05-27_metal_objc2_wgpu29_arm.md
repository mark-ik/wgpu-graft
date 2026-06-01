# TODO (macOS only): Metal objc2 arm for the wgpu-29 build

**Do this on the iMac.** It cannot be written or verified on Windows/Linux:
the code is `cfg(target_vendor = "apple")` and the wgpu-29 path uses objc2-metal
FFI that only compiles on macOS.

## Background

`grafting` now builds against two wgpu majors via cargo features (`wgpu-28`,
`wgpu-29`, default `wgpu-29`); see the `extern crate … as wgpu;` alias block in
`grafting/src/lib.rs`. The Vulkan, DX12, and GL paths were verified on Windows
under both features. **Metal was left untouched and is currently wgpu-28-shaped.**

The reason: wgpu migrated its Metal HAL backend from the `metal` crate (metal-rs)
to `objc2-metal` between hal 28 and hal 29.

- wgpu-hal **28**: `metal` crate; `wgpu::hal::metal::Device::texture_from_raw`
  takes a metal-rs `metal::Texture`.
- wgpu-hal **29**: `objc2_metal`; Metal texture type is
  `Retained<ProtocolObject<dyn MTLTexture>>`, and the old `texture_from_raw`
  shape is gone (renamed/changed — confirm the actual hal-29 entry point).

grafting's current Metal code (`metal_texture_ref.rs`, `raw_gl/metal.rs`) uses
metal-rs (`metal::Texture::from_ptr`, `wgpu::hal::metal::Device::texture_from_raw`,
`metal::MTLTextureType::D2`). That matches hal **28**, not hal 29, so the
`wgpu-29` Apple build does not compile as written. This is a pre-existing gap,
not introduced by the multi-version change.

## Tasks

1. **Find the hal-29 raw-texture entry point.** On the iMac, read the metal
   backend source for the resolved hal-29 version (e.g.
   `~/.cargo/registry/src/*/wgpu-hal-29.0.*/src/metal/mod.rs`) or
   `cargo doc -p wgpu-hal --features metal --open`. Locate the `Device` method
   that wraps an existing `MTLTexture` into a hal texture, and note its exact
   signature and the objc2-metal texture type it expects.

2. **`cfg`-fork the two Metal import functions** in
   `grafting/src/metal_texture_ref.rs` and `grafting/src/raw_gl/metal.rs`:
   - `#[cfg(feature = "wgpu-28")]` arm: the current metal-rs code, unchanged.
   - `#[cfg(all(feature = "wgpu-29", not(feature = "wgpu-28")))]` arm: new
     objc2-metal code. Wrap the raw `*mut c_void` MTLTexture as
     `Retained<ProtocolObject<dyn MTLTexture>>` and hand it to the hal-29 entry
     point found in step 1, then `create_texture_from_hal::<wgpu::wgc::api::Metal>`.

3. **Feature-gate the Apple deps** in `grafting/Cargo.toml` (currently all
   unconditional under `cfg(target_vendor = "apple")`):
   - `metal` (and `foreign-types-shared` if only used by the metal-rs arm) →
     pull only under `wgpu-28`.
   - `objc2-metal` / `objc2-foundation` / `objc2-io-surface` → keep for
     `wgpu-29` (note `objc2` itself is used by both arms for `Retained`).
   Wire these into the feature list the same way the wgpu deps are
   (`wgpu-28 = [… , "dep:metal"]`, etc.).

4. **Verify on macOS** (both must pass):
   - `cargo check -p grafting` (default = wgpu-29)
   - `cargo check -p grafting --no-default-features --features wgpu-28,surfman`
   Then build a Servo demo on macOS to exercise the IOSurface/Metal import at
   runtime.

## Notes

- The xilem zero-copy work targets the **wgpu-28** path (masonry-main is wgpu 28),
  so on Apple the xilem demo will exercise the metal-rs arm, which already
  matches hal 28. The wgpu-29 objc2 arm is what unblocks the **gpui** demo
  (glass-gpui is wgpu 29) on macOS.
- Keep both arms; do not delete the metal-rs path. It is the correct
  implementation for any wgpu-28 consumer.
