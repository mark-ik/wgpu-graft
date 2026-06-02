//! WIP scaffold — zero-copy Servo demo for Bevy (0.19.0-rc.2, wgpu 29).
//!
//! The full implementation is planned in
//! `docs/2026-06-02_bevy_gpui_zero_copy_plan.md`, which records the verified
//! Bevy 0.19 API: Servo as a `NonSend` resource in the main world exports a
//! D3D12 shared handle; an `ExtractSchedule` system carries it to the render
//! world; a `RenderSet::Prepare` system imports it onto Bevy's `RenderDevice`
//! and injects a `GpuImage` (`RenderAssets::<GpuImage>::insert`) for a
//! fullscreen `Sprite`'s `Handle<Image>`. `WgpuSettings` forces DX12.
//!
//! `keyutils` is staged for input forwarding once the render path lands.
#![allow(dead_code)]

mod keyutils;

fn main() {
    eprintln!(
        "demo-servo-bevy is a WIP scaffold; see \
         docs/2026-06-02_bevy_gpui_zero_copy_plan.md"
    );
}
