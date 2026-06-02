//! Demo embedding Servo in an iced (0.15-dev) application, zero-copy.
//!
//! Servo renders offscreen via ANGLE/surfman. Each frame the producer exports a
//! D3D12 **shared NT handle** for the rendered texture; iced's `shader` widget
//! imports that handle onto iced's *own* wgpu device and samples it directly. No
//! CPU readback.
//!
//! Why the shared-handle path (vs the in-process GL import the winit/egui demos
//! use): iced owns its wgpu device and only exposes it on the render thread,
//! inside the `shader` widget's `Primitive` — which must be `Send`. Servo's
//! surfman/GL context is not `Send` and lives on the main thread, so it cannot
//! ride along into the primitive. Instead the main thread exports a `Send` NT
//! handle (`Dx12SharedTexture`) and the primitive opens it on iced's device.
//!
//! Windows + DX12 only (the shared handle is a D3D12 resource). The iced UI adds
//! a URL bar above the Servo viewport; input is forwarded so pages stay
//! interactive.
//!
//! Usage:
//!   cargo run -p demo-servo-iced -- https://example.com
//!   cargo run -p demo-servo-iced -- servo.org        # auto-prefixes https://
//!   cargo run -p demo-servo-iced                     # built-in fixture page

mod keyutils;

use std::ffi::c_void;
use std::rc::Rc;
use std::time::Duration;

use demo_support::{DemoStatus, RenderPath};
use euclid::Scale;
use grafting::{Dx12SharedTexture, HostWgpuContext, SyncMechanism, import_dx12_shared_texture};
use iced::wgpu;
use iced::widget::shader::{Pipeline, Primitive};
use iced::widget::{column, shader, text, text_input};
use iced::{Element, Event, Length, Rectangle, Size, Subscription, Task, event, keyboard, mouse, window};
use rustls::crypto::aws_lc_rs;
use servo::{
    DevicePoint, EventLoopWaker, InputEvent, KeyState, MouseButton as ServoMouseButton,
    MouseButtonAction, MouseButtonEvent, MouseLeftViewportEvent, MouseMoveEvent, Servo,
    ServoBuilder, WebView, WebViewBuilder, WebViewDelegate, WheelDelta, WheelEvent, WheelMode,
};
use servo_wgpu_interop_adapter::ServoWgpuRenderingContext;
use url::Url;
use winit::dpi::PhysicalSize;

// ── Constants ────────────────────────────────────────────────────────────────

/// Estimated URL-bar height in logical pixels. Used to translate window
/// coordinates into Servo viewport coordinates and to size the Servo surface.
const NAV_BAR_HEIGHT: f32 = 50.0;

const DEFAULT_WIDTH: f32 = 1280.0;
const DEFAULT_HEIGHT: f32 = 800.0;

// ── App state ────────────────────────────────────────────────────────────────

struct AppState {
    servo: Servo,
    webview: WebView,
    render_ctx: Rc<ServoWgpuRenderingContext>,

    url_input: String,
    /// The most recent exported shared-handle descriptor (Send-safe), handed to
    /// the `shader` widget to import on iced's device.
    latest_frame: Option<SharedFrameDesc>,
    status: DemoStatus,
    viewport_size: Size,

    cursor_position: iced::Point,
    cursor_in_viewport: bool,
}

/// A `Send`-safe description of the current exported frame. The raw NT handle is
/// carried as a `u64` so the (non-`Send`) producer stays on the main thread
/// while the `Send` shader primitive opens the handle on iced's device.
#[derive(Clone, Copy, Debug)]
struct SharedFrameDesc {
    handle: u64,
    width: u32,
    height: u32,
    generation: u64,
}

// ── Messages ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Message {
    Tick,
    UrlInputChanged(String),
    Navigate,
    IcedEvent(Event),
}

// ── Boot ─────────────────────────────────────────────────────────────────────

fn boot() -> (AppState, Task<Message>) {
    let initial_url = demo_support::resolve_initial_url(env!("CARGO_MANIFEST_DIR"))
        .expect("failed to resolve initial URL");

    let viewport_w = DEFAULT_WIDTH;
    let viewport_h = DEFAULT_HEIGHT - NAV_BAR_HEIGHT;
    let size = PhysicalSize::new(viewport_w as u32, viewport_h as u32);

    let render_ctx = Rc::new(
        create_luid_anchored_context(size).expect("failed to create rendering context"),
    );

    let servo = ServoBuilder::default()
        .event_loop_waker(Box::new(NoopWaker))
        .build();
    servo.setup_logging();

    let webview = WebViewBuilder::new(&servo, render_ctx.clone())
        .url(initial_url.clone())
        .hidpi_scale_factor(Scale::new(1.0))
        .delegate(Rc::new(DemoDelegate))
        .build();

    let state = AppState {
        servo,
        webview,
        render_ctx,
        url_input: initial_url.to_string(),
        latest_frame: None,
        status: DemoStatus::new(RenderPath::GpuImport),
        viewport_size: Size::new(viewport_w, viewport_h),
        cursor_position: iced::Point::ORIGIN,
        cursor_in_viewport: false,
    };

    (state, Task::none())
}

/// Build a Servo rendering context whose surfman/ANGLE GPU matches the GPU iced
/// will pick. iced selects the **HighPerformance** adapter on **DX12** (we force
/// the backend in `main`). We create a throwaway HighPerformance-DX12 wgpu device
/// here purely to read that adapter's LUID and anchor surfman to it via
/// `new_for_device`; iced then creates its own device on the same physical GPU,
/// so the shared handle stays single-GPU (cross-GPU sharing garbles → flicker).
fn create_luid_anchored_context(
    size: PhysicalSize<u32>,
) -> Result<ServoWgpuRenderingContext, grafting::InteropError> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .expect("no DX12 adapter for LUID anchoring");
    let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("servo-iced-luid-anchor"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .expect("failed to create LUID-anchor device");

    // Anchors surfman to `device`'s GPU; the throwaway device is then dropped.
    ServoWgpuRenderingContext::new_for_device(size, &device)
}

// ── Update ───────────────────────────────────────────────────────────────────

fn update(state: &mut AppState, message: Message) -> Task<Message> {
    match message {
        Message::Tick => {
            state.servo.spin_event_loop();
            state.webview.paint();

            match state.render_ctx.current_dx12_shared_texture() {
                Ok(frame) => {
                    state
                        .status
                        .set_frame(RenderPath::GpuImport, frame.size.width, frame.size.height);
                    state.latest_frame = Some(SharedFrameDesc {
                        handle: frame.handle as u64,
                        width: frame.size.width,
                        height: frame.size.height,
                        generation: frame.generation,
                    });
                }
                Err(e) => eprintln!("[iced] shared-texture export failed: {e:?}"),
            }
        }

        Message::UrlInputChanged(url) => state.url_input = url,

        Message::Navigate => {
            let raw = &state.url_input;
            match Url::parse(raw).or_else(|_| Url::parse(&format!("https://{raw}"))) {
                Ok(url) => state.webview.load(url),
                Err(_) => eprintln!("invalid URL: {raw}"),
            }
        }

        Message::IcedEvent(event) => handle_event(state, event),
    }

    Task::none()
}

fn handle_event(state: &mut AppState, event: Event) {
    match event {
        Event::Window(window::Event::Resized(new_size)) => {
            let vp_w = new_size.width;
            let vp_h = (new_size.height - NAV_BAR_HEIGHT).max(1.0);
            state.viewport_size = Size::new(vp_w, vp_h);
            state.webview.resize(PhysicalSize::new(vp_w as u32, vp_h as u32));
        }

        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            state.cursor_position = position;
            let was_in = state.cursor_in_viewport;
            state.cursor_in_viewport = position.y >= NAV_BAR_HEIGHT;

            if state.cursor_in_viewport {
                let pt = servo_point(position);
                state
                    .webview
                    .notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(
                        servo::WebViewPoint::Device(pt),
                    )));
            } else if was_in {
                state
                    .webview
                    .notify_input_event(InputEvent::MouseLeftViewport(
                        MouseLeftViewportEvent::default(),
                    ));
            }
        }

        Event::Mouse(mouse::Event::CursorLeft) => {
            if state.cursor_in_viewport {
                state
                    .webview
                    .notify_input_event(InputEvent::MouseLeftViewport(
                        MouseLeftViewportEvent::default(),
                    ));
                state.cursor_in_viewport = false;
            }
        }

        Event::Mouse(mouse::Event::ButtonPressed(btn)) if state.cursor_in_viewport => {
            if let Some(servo_btn) = map_mouse_button(btn) {
                let pt = servo_point(state.cursor_position);
                state
                    .webview
                    .notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                        MouseButtonAction::Down,
                        servo_btn,
                        servo::WebViewPoint::Device(pt),
                    )));
            }
        }

        Event::Mouse(mouse::Event::ButtonReleased(btn)) if state.cursor_in_viewport => {
            if let Some(servo_btn) = map_mouse_button(btn) {
                let pt = servo_point(state.cursor_position);
                state
                    .webview
                    .notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                        MouseButtonAction::Up,
                        servo_btn,
                        servo::WebViewPoint::Device(pt),
                    )));
            }
        }

        Event::Mouse(mouse::Event::WheelScrolled { delta }) if state.cursor_in_viewport => {
            let (dx, dy, mode) = match delta {
                mouse::ScrollDelta::Lines { x, y } => {
                    ((x as f64) * 38.0, (y as f64) * 38.0, WheelMode::DeltaLine)
                }
                mouse::ScrollDelta::Pixels { x, y } => (x as f64, y as f64, WheelMode::DeltaPixel),
            };
            let pt = servo_point(state.cursor_position);
            state
                .webview
                .notify_input_event(InputEvent::Wheel(WheelEvent::new(
                    WheelDelta {
                        x: dx,
                        y: dy,
                        z: 0.0,
                        mode,
                    },
                    servo::WebViewPoint::Device(pt),
                )));
        }

        Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key,
            location,
            modifiers,
            repeat,
            ..
        }) => {
            let kbd = keyutils::keyboard_event_from_iced(
                KeyState::Down,
                &key,
                physical_key,
                location,
                modifiers,
                repeat,
            );
            state.webview.notify_input_event(InputEvent::Keyboard(kbd));
        }

        Event::Keyboard(keyboard::Event::KeyReleased {
            key,
            physical_key,
            location,
            modifiers,
            ..
        }) => {
            let kbd = keyutils::keyboard_event_from_iced(
                KeyState::Up,
                &key,
                physical_key,
                location,
                modifiers,
                false,
            );
            state.webview.notify_input_event(InputEvent::Keyboard(kbd));
        }

        _ => {}
    }
}

// ── View ─────────────────────────────────────────────────────────────────────

fn view(state: &AppState) -> Element<'_, Message> {
    let url_bar = text_input("Enter URL...", &state.url_input)
        .on_input(Message::UrlInputChanged)
        .on_submit(Message::Navigate);

    let content: Element<Message> = if let Some(desc) = state.latest_frame {
        shader(ServoProgram { desc })
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    } else {
        text("Servo loading…")
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    };

    column![url_bar, text(state.status.summary()), content].into()
}

// ── Subscription ─────────────────────────────────────────────────────────────

fn subscription(_state: &AppState) -> Subscription<Message> {
    Subscription::batch([
        iced::time::every(Duration::from_millis(16)).map(|_| Message::Tick),
        event::listen_with(filter_events),
    ])
}

fn filter_events(event: Event, _status: event::Status, _window: window::Id) -> Option<Message> {
    match &event {
        Event::Mouse(_) | Event::Keyboard(_) => Some(Message::IcedEvent(event)),
        Event::Window(window::Event::Resized(_)) => Some(Message::IcedEvent(event)),
        _ => None,
    }
}

// ── Shader widget: zero-copy Servo frame ─────────────────────────────────────

/// The `shader::Program` carrying the latest exported frame descriptor.
#[derive(Debug)]
struct ServoProgram {
    desc: SharedFrameDesc,
}

impl<Message> shader::Program<Message> for ServoProgram {
    type State = ();
    type Primitive = ServoPrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        ServoPrimitive { desc: self.desc }
    }
}

#[derive(Debug)]
struct ServoPrimitive {
    desc: SharedFrameDesc,
}

impl Primitive for ServoPrimitive {
    type Pipeline = ServoPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _bounds: &Rectangle,
        _viewport: &shader::Viewport,
    ) {
        pipeline.ensure_import(device, queue, &self.desc);
    }

    fn draw(&self, pipeline: &Self::Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        pipeline.draw(render_pass)
    }
}

/// Per-`Primitive`-type GPU state: the sampling pipeline plus the imported
/// shared texture (cached by handle + size; re-imported only on change).
#[derive(Debug)]
struct ServoPipeline {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    bind_group_layout: wgpu::BindGroupLayout,
    cached: Option<CachedImport>,
}

#[derive(Debug)]
struct CachedImport {
    handle: u64,
    size: (u32, u32),
    /// Keeps the imported (aliasing) texture alive for the bind group.
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

impl Pipeline for ServoPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("servo-iced-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_WGSL.into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("servo-iced-bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("servo-iced-pl"),
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("servo-iced-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("servo-iced-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            pipeline,
            sampler,
            bind_group_layout,
            cached: None,
        }
    }
}

impl ServoPipeline {
    /// Import the shared handle onto iced's device, caching by (handle, size).
    /// The imported texture aliases the producer's live D3D11 texture, so its
    /// content updates each frame without re-importing.
    fn ensure_import(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        desc: &SharedFrameDesc,
    ) {
        let want = (desc.handle, (desc.width, desc.height));
        let have = self.cached.as_ref().map(|c| (c.handle, c.size));
        if have == Some(want) {
            return;
        }

        let frame = Dx12SharedTexture {
            size: PhysicalSize::new(desc.width, desc.height),
            format: wgpu::TextureFormat::Rgba8Unorm,
            generation: desc.generation,
            producer_sync: SyncMechanism::None,
            fence_value: 0,
            handle: desc.handle as *mut c_void,
        };
        let host = HostWgpuContext::new(device.clone(), queue.clone());
        match import_dx12_shared_texture(&frame, &host) {
            Ok(texture) => {
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("servo-iced-bind-group"),
                    layout: &self.bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
                self.cached = Some(CachedImport {
                    handle: desc.handle,
                    size: (desc.width, desc.height),
                    _texture: texture,
                    bind_group,
                });
            }
            Err(e) => eprintln!("[iced] import_dx12_shared_texture failed: {e:?}"),
        }
    }

    fn draw(&self, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        let Some(cached) = &self.cached else {
            return false;
        };
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &cached.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
        true
    }
}

const SHADER_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    // Fullscreen triangle; uv in [0,2], visible region maps to uv in [0,1].
    let uv = vec2<f32>(f32((vid << 1u) & 2u), f32(vid & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    // The imported Servo texture is bottom-left origin; with wgpu's top-left
    // framebuffer/texture convention this unflipped uv displays it upright.
    out.uv = uv;
    return out;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv);
}
"#;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Convert an iced Point (logical window coords) to a Servo DevicePoint by
/// subtracting the nav bar height.
fn servo_point(position: iced::Point) -> DevicePoint {
    DevicePoint::new(position.x, (position.y - NAV_BAR_HEIGHT).max(0.0))
}

fn map_mouse_button(btn: mouse::Button) -> Option<ServoMouseButton> {
    Some(match btn {
        mouse::Button::Left => ServoMouseButton::Left,
        mouse::Button::Right => ServoMouseButton::Right,
        mouse::Button::Middle => ServoMouseButton::Middle,
        mouse::Button::Back => ServoMouseButton::Back,
        mouse::Button::Forward => ServoMouseButton::Forward,
        mouse::Button::Other(v) => ServoMouseButton::Other(v),
    })
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

    fn notify_closed(&self, _webview: WebView) {
        println!("[servo] webview closed");
    }

    fn notify_crashed(&self, _webview: WebView, reason: String, backtrace: Option<String>) {
        eprintln!("[servo] CRASH: {reason}");
        if let Some(bt) = backtrace {
            eprintln!("{bt}");
        }
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() -> iced::Result {
    // Force iced onto the DX12 backend so it picks the same physical GPU the
    // shared handle is created on (the D3D12 OpenSharedHandle import requires
    // DX12). iced reads `WGPU_BACKEND` via `wgpu::Backends::from_env`.
    // SAFETY: set before any threads/wgpu init.
    unsafe {
        std::env::set_var("WGPU_BACKEND", "dx12");
    }

    aws_lc_rs::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    iced::application(boot, update, view)
        .title("demo-servo-iced")
        .subscription(subscription)
        .window_size((DEFAULT_WIDTH, DEFAULT_HEIGHT))
        .run()
}
