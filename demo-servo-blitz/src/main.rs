//! Demo embedding Servo via the Blitz renderer stack (`anyrender_vello`),
//! zero-copy.
//!
//! Blitz renders through `anyrender_vello` → [vello] (wgpu 29). This demo drives
//! a [`VelloWindowRenderer`] (the renderer Blitz itself uses), takes its wgpu
//! device, runs Servo **in-process** onto that device, imports each rendered
//! frame zero-copy as a `wgpu::Texture`, registers it with vello via
//! `try_register_custom_resource`, and fills the window with it in the scene.
//!
//! Because we hold anyrender's device on the main thread, the simple in-process
//! GL import is used (no shared handle). Mouse/scroll/keyboard are forwarded to
//! Servo. Windows + DX12 (the import path is ANGLE-D3D11 → DX12).
//!
//! Usage:
//!   cargo run -p demo-servo-blitz -- https://example.com
//!   cargo run -p demo-servo-blitz -- servo.org        # auto-prefixes https://
//!   cargo run -p demo-servo-blitz                     # built-in fixture page

use std::{rc::Rc, sync::Arc};

use anyrender::{Paint, PaintScene, RenderContext, WindowRenderer};
use anyrender_vello::VelloWindowRenderer;
use demo_support::{DemoStatus, RenderPath};
use euclid::Scale;
use rustls::crypto::aws_lc_rs;
use servo::{
    DevicePoint, EventLoopWaker, InputEvent, MouseButton as ServoMouseButton, MouseButtonAction,
    MouseButtonEvent, MouseLeftViewportEvent, MouseMoveEvent, Servo, ServoBuilder, WebView,
    WebViewBuilder, WebViewDelegate, WheelDelta, WheelEvent, WheelMode,
};
use servo_wgpu_interop_adapter::ServoWgpuInteropAdapter;
use url::Url;
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Fill, ImageBrush, ImageSampler};
use winit::{
    application::ApplicationHandler,
    dpi::{PhysicalPosition, PhysicalSize},
    event::{ElementState, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    keyboard::ModifiersState,
    window::Window,
};

mod keyutils;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Force anyrender (via wgpu_context's `Backends::from_env`) onto DX12 so the
    // ANGLE-D3D11 → DX12 shared-texture import path works and the surfman/ANGLE
    // GPU can be LUID-matched to anyrender's wgpu device.
    // SAFETY: set before any wgpu/thread init.
    unsafe {
        std::env::set_var("WGPU_BACKEND", "dx12");
    }

    aws_lc_rs::default_provider()
        .install_default()
        .expect("failed to install rustls aws-lc provider");

    let event_loop = EventLoop::with_user_event()
        .build()
        .expect("failed to create event loop");
    let initial_url = demo_support::resolve_initial_url(env!("CARGO_MANIFEST_DIR"))?;
    let mut app = App::new(&event_loop, initial_url);
    Ok(event_loop.run_app(&mut app)?)
}

struct App {
    state: AppStage,
}

enum AppStage {
    Initial { initial_url: Url, waker: AppWaker },
    Running(AppState),
}

struct AppState {
    window: Arc<Window>,
    servo: Servo,
    webview: WebView,
    interop: ServoWgpuInteropAdapter,
    renderer: VelloWindowRenderer,
    status: DemoStatus,
    cursor_position: PhysicalPosition<f64>,
    modifiers: ModifiersState,
}

impl App {
    fn new(event_loop: &EventLoop<WakerEvent>, initial_url: Url) -> Self {
        Self {
            state: AppStage::Initial {
                initial_url,
                waker: AppWaker::new(event_loop),
            },
        }
    }
}

impl ApplicationHandler<WakerEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let AppStage::Initial { initial_url, waker } = &self.state else {
            return;
        };

        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("demo-servo-blitz")
                        .with_inner_size(PhysicalSize::new(1280, 800)),
                )
                .expect("failed to create window"),
        );

        let size = window.inner_size();

        // Bring up the Blitz/vello renderer and grab its wgpu device. On native,
        // resume() runs the wgpu init inline (pollster), so complete_resume()
        // succeeds immediately.
        let mut renderer = VelloWindowRenderer::new();
        renderer.resume(window.clone(), size.width.max(1), size.height.max(1), || {});
        if !renderer.complete_resume() {
            panic!("vello renderer failed to become active");
        }
        let device_handle = renderer
            .current_device_handle()
            .expect("device handle after resume");
        let device = device_handle.device.clone();
        let queue = device_handle.queue.clone();

        // Servo renders in-process onto anyrender's own wgpu device. The adapter
        // LUID-matches surfman/ANGLE to that device (DX12).
        let interop = ServoWgpuInteropAdapter::new(device, queue, size)
            .expect("failed to create Servo interop adapter");

        let servo = ServoBuilder::default()
            .event_loop_waker(Box::new(waker.clone()))
            .build();
        servo.setup_logging();

        let webview = WebViewBuilder::new(&servo, interop.rendering_context())
            .url(initial_url.clone())
            .hidpi_scale_factor(Scale::new(window.scale_factor() as f32))
            .delegate(Rc::new(RedrawDelegate {
                window: window.clone(),
            }))
            .build();

        let status =
            DemoStatus::new(RenderPath::GpuImport).with_backend("DX12 (vello)".to_string());

        println!("[blitz] vello renderer active; Servo embedded zero-copy");
        window.request_redraw();

        self.state = AppStage::Running(AppState {
            window,
            servo,
            webview,
            interop,
            renderer,
            status,
            cursor_position: PhysicalPosition::new(0.0, 0.0),
            modifiers: ModifiersState::default(),
        });
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: WakerEvent) {
        if let AppStage::Running(state) = &mut self.state {
            state.servo.spin_event_loop();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let AppStage::Running(state) = &mut self.state else {
            return;
        };

        state.servo.spin_event_loop();

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::RedrawRequested => {
                state.render_frame();
            }

            WindowEvent::Resized(new_size) => {
                state
                    .renderer
                    .set_size(new_size.width.max(1), new_size.height.max(1));
                // `webview.resize` is the sole driver of the Servo-side resize
                // (see the winit demo: do not also pre-resize the rendering
                // context, or Servo early-returns before updating the rect).
                state.webview.resize(new_size);
                state.window.request_redraw();
            }

            WindowEvent::CursorMoved { position, .. } => {
                state.cursor_position = position;
                let point = DevicePoint::new(position.x as f32, position.y as f32);
                state
                    .webview
                    .notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(
                        servo::WebViewPoint::Device(point),
                    )));
            }

            WindowEvent::CursorLeft { .. } => {
                state
                    .webview
                    .notify_input_event(InputEvent::MouseLeftViewport(
                        MouseLeftViewportEvent::default(),
                    ));
            }

            WindowEvent::MouseInput {
                state: btn_state,
                button,
                ..
            } => {
                let servo_button = match button {
                    winit::event::MouseButton::Left => ServoMouseButton::Left,
                    winit::event::MouseButton::Right => ServoMouseButton::Right,
                    winit::event::MouseButton::Middle => ServoMouseButton::Middle,
                    _ => return,
                };
                let action = match btn_state {
                    ElementState::Pressed => MouseButtonAction::Down,
                    ElementState::Released => MouseButtonAction::Up,
                };
                let pos = state.cursor_position;
                let point = DevicePoint::new(pos.x as f32, pos.y as f32);
                state
                    .webview
                    .notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                        action,
                        servo_button,
                        servo::WebViewPoint::Device(point),
                    )));
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy, mode) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        ((x as f64) * 38.0, (y as f64) * 38.0, WheelMode::DeltaLine)
                    }
                    MouseScrollDelta::PixelDelta(pos) => (pos.x, pos.y, WheelMode::DeltaPixel),
                };
                let pos = state.cursor_position;
                let point = DevicePoint::new(pos.x as f32, pos.y as f32);
                state
                    .webview
                    .notify_input_event(InputEvent::Wheel(WheelEvent::new(
                        WheelDelta {
                            x: dx,
                            y: dy,
                            z: 0.0,
                            mode,
                        },
                        servo::WebViewPoint::Device(point),
                    )));
            }

            WindowEvent::ModifiersChanged(mods) => {
                state.modifiers = mods.state();
            }

            WindowEvent::KeyboardInput { event, .. } => {
                let kbd = keyutils::keyboard_event_from_winit(&event, state.modifiers);
                state.webview.notify_input_event(InputEvent::Keyboard(kbd));
            }

            _ => {}
        }
    }
}

impl AppState {
    fn render_frame(&mut self) {
        self.webview.paint();

        // In-process zero-copy import onto anyrender's device. Default options
        // normalize to a top-left `Rgba8Unorm` texture (with COPY_SRC), which is
        // exactly what vello's `register_texture` needs.
        let imported = match self.interop.import_current_frame_default() {
            Ok(imported) => imported,
            Err(e) => {
                eprintln!("[blitz] GPU import failed: {e:?}");
                return;
            }
        };
        let frame_size = imported.size;
        self.status
            .set_frame(RenderPath::GpuImport, frame_size.width, frame_size.height);
        self.window
            .set_title(&format!("demo-servo-blitz — {}", self.status.summary()));

        // Register the imported texture with vello and fill the window with it.
        // The normalizer hands back a fresh texture each frame, so register and
        // unregister per frame (vello copies it into its atlas on render).
        let resource_id = match self
            .renderer
            .try_register_custom_resource(Box::new(imported.texture))
        {
            Ok(id) => id,
            Err(e) => {
                eprintln!("[blitz] register_custom_resource failed: {e:?}");
                return;
            }
        };

        let win = self.window.inner_size();
        let (w, h) = (win.width.max(1) as f64, win.height.max(1) as f64);
        let (iw, ih) = (
            frame_size.width.max(1) as f64,
            frame_size.height.max(1) as f64,
        );
        // Scale the (top-left) Servo image to fill the window. No Y-flip: the
        // normalized texture is already top-left, like the egui demo.
        let brush_transform = Affine::scale_non_uniform(w / iw, h / ih);
        let paint = Paint::Resource(ImageBrush {
            image: resource_id,
            sampler: ImageSampler::default(),
        });

        self.renderer.render(|scene| {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                &paint,
                Some(brush_transform),
                &Rect::new(0.0, 0.0, w, h),
            );
        });

        self.renderer.unregister_resource(resource_id);
        self.window.request_redraw();
    }
}

// ── Waker ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppWaker {
    proxy: EventLoopProxy<WakerEvent>,
}

#[derive(Debug)]
struct WakerEvent;

impl AppWaker {
    fn new(event_loop: &EventLoop<WakerEvent>) -> Self {
        Self {
            proxy: event_loop.create_proxy(),
        }
    }
}

impl EventLoopWaker for AppWaker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        let _ = self.proxy.send_event(WakerEvent);
    }
}

// ── WebView delegate ────────────────────────────────────────────────────────

struct RedrawDelegate {
    window: Arc<Window>,
}

impl WebViewDelegate for RedrawDelegate {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        self.window.request_redraw();
    }

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
