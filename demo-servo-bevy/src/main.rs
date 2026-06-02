//! Demo embedding Servo in a [Bevy] app, zero-copy.
//!
//! Bevy's render world runs on a separate thread, and Servo's surfman/GL context
//! is `!Send`, so they can't share the import in-process. Instead this uses the
//! shared-handle seam (the same reason the iced demo needs it):
//!
//! - Servo lives in the **main world** as a `NonSend` resource. A main-world
//!   system paints it and exports a D3D12 shared NT handle (a `Send` `u64`).
//! - An `ExtractSchedule` system carries the handle into the **render world**.
//! - A render-world system (after `PrepareAssets`, before `Queue`) opens the
//!   handle on Bevy's `RenderDevice` and injects a `GpuImage` into
//!   `RenderAssets<GpuImage>` for a fullscreen `Sprite`'s `Handle<Image>`.
//!
//! surfman/ANGLE is LUID-anchored to a throwaway HighPerformance-DX12 device and
//! Bevy is forced to DX12 + HighPerformance, so both land on the same GPU.
//! Windows + DX12 only.

#![allow(clippy::type_complexity)]

use std::ffi::c_void;

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureUsages,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::settings::{Backends, PowerPreference, RenderCreation, WgpuSettings};
use bevy::render::texture::GpuImage;
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderPlugin, RenderSystems};
use bevy::window::{PrimaryWindow, WindowResolution};
use grafting::{Dx12SharedTexture, HostWgpuContext, SyncMechanism, import_dx12_shared_texture};
use rustls::crypto::aws_lc_rs;
use servo::{
    EventLoopWaker, Servo, ServoBuilder, WebView, WebViewBuilder, WebViewDelegate,
};
use servo_wgpu_interop_adapter::ServoWgpuInteropAdapter;
use url::Url;
use winit::dpi::PhysicalSize;

const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 800;

// ── Servo side (main world, !Send) ───────────────────────────────────────────

/// Non-`Send` Servo state, held as a `NonSend` resource on the main thread.
struct ServoState {
    servo: Servo,
    webview: WebView,
    interop: ServoWgpuInteropAdapter,
    size: PhysicalSize<u32>,
}

/// The latest exported shared-handle frame (main world). `Send`.
#[derive(Resource, Default, Clone, Copy)]
struct ServoFrame(Option<FrameDesc>);

#[derive(Clone, Copy)]
struct FrameDesc {
    handle: u64,
    width: u32,
    height: u32,
    generation: u64,
}

/// The placeholder image the Servo frame is injected into (main world).
#[derive(Resource, Clone)]
struct ServoImage(Handle<Image>);

// ── Render world mirrors (filled by ExtractSchedule) ─────────────────────────

#[derive(Resource, Default, Clone, Copy)]
struct ExtractedFrame(Option<FrameDesc>);

#[derive(Resource, Default, Clone, Copy)]
struct ExtractedImageId(Option<AssetId<Image>>);

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    aws_lc_rs::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let initial_url = demo_support::resolve_initial_url(env!("CARGO_MANIFEST_DIR"))
        .expect("failed to resolve initial URL");

    // Anchor surfman/ANGLE to a HighPerformance-DX12 GPU and run Servo on it.
    // Bevy (forced to DX12 + HighPerformance below) lands on the same GPU, so the
    // shared handle opened on Bevy's RenderDevice stays single-GPU.
    let servo_state = build_servo(&initial_url);

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "demo-servo-bevy".into(),
                    resolution: WindowResolution::new(DEFAULT_WIDTH, DEFAULT_HEIGHT),
                    ..default()
                }),
                ..default()
            })
            .set(RenderPlugin {
                // Force DX12 + HighPerformance so Bevy's GPU matches surfman's.
                render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                    backends: Some(Backends::DX12),
                    power_preference: PowerPreference::HighPerformance,
                    ..default()
                })),
                ..default()
            }),
    );

    app.insert_non_send(servo_state)
        .init_resource::<ServoFrame>()
        .add_systems(Startup, setup)
        .add_systems(Update, (drive_servo, resize_servo_image, fit_sprite_to_window));

    // Render world: extract the handle, then inject the imported texture.
    let render_app = app.sub_app_mut(RenderApp);
    render_app
        .init_resource::<ExtractedFrame>()
        .init_resource::<ExtractedImageId>()
        .add_systems(ExtractSchedule, extract_servo_frame)
        .add_systems(
            Render,
            inject_servo_image
                .after(RenderSystems::PrepareAssets)
                .before(RenderSystems::Queue),
        );

    app.run();
}

fn build_servo(initial_url: &Url) -> ServoState {
    let size = PhysicalSize::new(DEFAULT_WIDTH, DEFAULT_HEIGHT);

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        flags: wgpu::InstanceFlags::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::default(),
        display: None,
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .expect("no DX12 adapter for LUID anchoring");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("servo-bevy-luid-anchor"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .expect("failed to create LUID-anchor device");

    let interop = ServoWgpuInteropAdapter::new(device, queue, size)
        .expect("failed to create Servo interop adapter");

    let servo = ServoBuilder::default()
        .event_loop_waker(Box::new(NoopWaker))
        .build();
    servo.setup_logging();

    let webview = WebViewBuilder::new(&servo, interop.rendering_context())
        .url(initial_url.clone())
        .hidpi_scale_factor(euclid::Scale::new(1.0))
        .delegate(std::rc::Rc::new(DemoDelegate))
        .build();

    ServoState {
        servo,
        webview,
        interop,
        size,
    }
}

// ── Startup: camera + fullscreen sprite on a placeholder image ───────────────

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    commands.spawn(Camera2d);

    // Bevy owns this texture; the render world COPIES the imported Servo frame
    // into it each frame (so the sprite's bind group aliases a stable Bevy
    // texture, not the short-lived shared-handle import). RENDER_WORLD only (no
    // CPU data); needs COPY_DST for the copy and TEXTURE_BINDING to be sampled.
    let mut placeholder = Image::new_uninit(
        Extent3d {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    placeholder.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
    let handle = images.add(placeholder);
    commands.insert_resource(ServoImage(handle.clone()));

    let size = windows
        .single()
        .map(|w| Vec2::new(w.width(), w.height()))
        .unwrap_or(Vec2::new(DEFAULT_WIDTH as f32, DEFAULT_HEIGHT as f32));

    commands.spawn(Sprite {
        image: handle,
        custom_size: Some(size),
        // The exported Servo texture is bottom-left origin; flip vertically so
        // the page displays upright.
        flip_y: true,
        ..default()
    });
}

// ── Update (main world): drive Servo, export the frame, fit the sprite ───────

fn drive_servo(
    mut servo: NonSendMut<ServoState>,
    mut frame: ResMut<ServoFrame>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    servo.servo.spin_event_loop();

    if let Ok(window) = windows.single() {
        let phys = window.resolution.physical_size();
        let new_size = PhysicalSize::new(phys.x.max(1), phys.y.max(1));
        if new_size != servo.size {
            servo.webview.resize(new_size);
            servo.size = new_size;
        }
    }

    servo.webview.paint();

    match servo
        .interop
        .rendering_context_handle()
        .current_dx12_shared_texture()
    {
        Ok(shared) => {
            frame.0 = Some(FrameDesc {
                handle: shared.handle as u64,
                width: shared.size.width,
                height: shared.size.height,
                generation: shared.generation,
            });
        }
        Err(e) => eprintln!("[bevy] shared-texture export failed: {e:?}"),
    }
}

/// Keep the Bevy-owned placeholder image sized to the Servo frame. Resizing the
/// asset fires an `AssetEvent::Modified`, which makes Bevy re-create the GpuImage
/// texture at the new size and refresh the sprite's image bind group (otherwise
/// the cached bind group would keep pointing at the old-size texture).
fn resize_servo_image(
    frame: Res<ServoFrame>,
    servo_image: Res<ServoImage>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(desc) = frame.0 else {
        return;
    };
    if let Some(mut image) = images.get_mut(&servo_image.0) {
        let size = image.texture_descriptor.size;
        if size.width != desc.width || size.height != desc.height {
            image.texture_descriptor.size = Extent3d {
                width: desc.width,
                height: desc.height,
                depth_or_array_layers: 1,
            };
        }
    }
}

fn fit_sprite_to_window(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut sprites: Query<&mut Sprite>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let size = Vec2::new(window.width(), window.height());
    for mut sprite in &mut sprites {
        if sprite.custom_size != Some(size) {
            sprite.custom_size = Some(size);
        }
    }
}

// ── Render world: extract + inject ───────────────────────────────────────────

fn extract_servo_frame(
    frame: Extract<Res<ServoFrame>>,
    image: Extract<Option<Res<ServoImage>>>,
    mut out_frame: ResMut<ExtractedFrame>,
    mut out_id: ResMut<ExtractedImageId>,
) {
    out_frame.0 = frame.0;
    out_id.0 = image.as_ref().map(|i| i.0.id());
}

fn inject_servo_image(
    frame: Res<ExtractedFrame>,
    image_id: Res<ExtractedImageId>,
    device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    gpu_images: Res<RenderAssets<GpuImage>>,
) {
    let (Some(desc), Some(id)) = (frame.0, image_id.0) else {
        return;
    };
    // Bevy's own GpuImage for the sprite. It lags a frame during resize, so only
    // copy when its size matches the freshly imported frame.
    let Some(gpu_image) = gpu_images.get(id) else {
        return;
    };
    if gpu_image.texture_descriptor.size.width != desc.width
        || gpu_image.texture_descriptor.size.height != desc.height
    {
        return;
    }

    let shared = Dx12SharedTexture {
        size: PhysicalSize::new(desc.width, desc.height),
        format: TextureFormat::Rgba8Unorm,
        generation: desc.generation,
        producer_sync: SyncMechanism::None,
        fence_value: 0,
        handle: desc.handle as *mut c_void,
    };
    let host = HostWgpuContext::new(device.wgpu_device().clone(), (**render_queue.0).clone());
    let imported = match import_dx12_shared_texture(&shared, &host) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[bevy] import_dx12_shared_texture failed: {e:?}");
            return;
        }
    };

    // Copy the imported (short-lived) alias into Bevy's stable owned texture, so
    // the sprite's cached bind group keeps sampling a texture that stays valid
    // across frames and resizes.
    let mut encoder =
        device
            .wgpu_device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("servo-bevy-copy"),
            });
    encoder.copy_texture_to_texture(
        imported.as_image_copy(),
        gpu_image.texture.as_image_copy(),
        Extent3d {
            width: desc.width,
            height: desc.height,
            depth_or_array_layers: 1,
        },
    );
    render_queue.submit([encoder.finish()]);
}

// ── Servo support ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct NoopWaker;

impl EventLoopWaker for NoopWaker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }
    fn wake(&self) {}
}

struct DemoDelegate;

impl WebViewDelegate for DemoDelegate {
    fn notify_url_changed(&self, _webview: WebView, url: Url) {
        println!("[servo] URL changed: {url}");
    }
    fn notify_crashed(&self, _webview: WebView, reason: String, backtrace: Option<String>) {
        eprintln!("[servo] CRASH: {reason}");
        if let Some(bt) = backtrace {
            eprintln!("{bt}");
        }
    }
}
