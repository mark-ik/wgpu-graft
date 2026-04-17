//! Demo embedding Servo in a Xilem app with input forwarding.
//!
//! Servo renders offscreen via surfman/GL into a `ServoWgpuRenderingContext`.
//! Each frame, the GPU pixels are read back to CPU as an `image::RgbaImage`,
//! converted to a `peniko::ImageData`, and injected into the Xilem view tree via
//! a `tokio::sync::watch` channel + Xilem `task_raw` view.
//!
//! The Xilem UI provides a URL bar and a Go button above the Servo viewport.
//! Mouse, scroll, and keyboard events in the viewport region are forwarded
//! to Servo so pages are interactive (links, scrolling, text input).
//!
//! Usage:
//!   cargo run -p demo-servo-xilem -- https://example.com
//!   cargo run -p demo-servo-xilem          # opens built-in fixture page

mod keyutils;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use euclid::Scale;
use masonry::peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
use masonry::theme::default_property_set;
use masonry_winit::app::{AppDriver, MasonryState, MasonryUserEvent};
use rustls::crypto::aws_lc_rs;
use servo::EventLoopWaker;
use servo::{
    DevicePoint, InputEvent, MouseButton as ServoMouseButton, MouseButtonAction, MouseButtonEvent,
    MouseLeftViewportEvent, MouseMoveEvent, RenderingContext, Servo, ServoBuilder, WebView,
    WebViewBuilder, WebViewDelegate, WheelDelta, WheelEvent, WheelMode,
};
use servo_wgpu_interop_adapter::ServoWgpuRenderingContext;
use tokio::sync::watch;
use url::Url;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::error::EventLoopError;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::ModifiersState;
use xilem::core::fork;
use xilem::view::{
    FlexExt, flex_col, flex_row, image as image_view, label, sized_box, task_raw, text_button,
    text_input,
};
use xilem::{EventLoop, WidgetView, WindowOptions, Xilem};

// ── App state ────────────────────────────────────────────────────────────────

struct AppState {
    /// The URL currently shown in the navigation bar input.
    nav_input: String,
    /// The last successfully read frame from Servo, or None before first paint.
    current_image: Option<ImageData>,
}

// ── View logic ────────────────────────────────────────────────────────────────

fn app_logic(
    state: &mut AppState,
    image_rx: watch::Receiver<Option<ImageData>>,
    nav_request: Arc<Mutex<Option<String>>>,
) -> impl WidgetView<AppState> + use<> {
    let nav_for_enter = nav_request.clone();
    let nav_for_go = nav_request.clone();
    let nav_bar = flex_row((
        text_input(
            state.nav_input.clone(),
            |state: &mut AppState, new_text: String| {
                state.nav_input = new_text;
            },
        )
        .on_enter(move |state: &mut AppState, _text: String| {
            *nav_for_enter.lock().unwrap() = Some(state.nav_input.clone());
        })
        .flex(1.0),
        text_button("Go", move |state: &mut AppState| {
            *nav_for_go.lock().unwrap() = Some(state.nav_input.clone());
        }),
    ));

    let content = if let Some(img) = state.current_image.clone() {
        xilem::core::one_of::OneOf2::A(
            sized_box(image_view(img).fit(xilem::view::ObjectFit::Fill))
                .expand_width()
                .expand_height(),
        )
    } else {
        xilem::core::one_of::OneOf2::B(
            sized_box(label("Servo loading…"))
                .expand_width()
                .expand_height(),
        )
    };

    // `task_raw` must take `Fn`, not `FnOnce`. Since watch::Receiver: Clone,
    // we clone it inside the closure body — the move-captured `rx` stays alive
    // for subsequent (no-op) calls while the async task owns the clone.
    fork(
        flex_col((nav_bar, content.flex(1.0))),
        task_raw(
            move |proxy| {
                let mut rx = image_rx.clone(); // borrow, then clone — Fn-safe
                async move {
                    loop {
                        if rx.changed().await.is_err() {
                            break;
                        }
                        if let Some(img) = rx.borrow_and_update().clone() {
                            if proxy.message(img).is_err() {
                                break;
                            }
                        }
                    }
                }
            },
            |state: &mut AppState, img: ImageData| {
                state.current_image = Some(img);
            },
        ),
    )
}

// ── Servo helpers ─────────────────────────────────────────────────────────────

struct ServoState {
    servo: Servo,
    webview: WebView,
    render_ctx: Rc<ServoWgpuRenderingContext>,
}

impl ServoState {
    fn init(size: PhysicalSize<u32>, initial_url: Url) -> Result<Self, String> {
        let render_ctx = Rc::new(
            ServoWgpuRenderingContext::new(size).map_err(|e| format!("surfman error: {e:?}"))?,
        );

        let servo = ServoBuilder::default()
            .event_loop_waker(Box::new(NoopWaker))
            .build();
        servo.setup_logging();

        let webview = WebViewBuilder::new(&servo, render_ctx.clone())
            .url(initial_url)
            .hidpi_scale_factor(Scale::new(1.0))
            .delegate(Rc::new(DemoDelegate))
            .build();

        Ok(Self {
            servo,
            webview,
            render_ctx,
        })
    }
}

/// A no-op Servo event loop waker: in Poll mode we don't need explicit waking.
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
        eprintln!("[servo] URL changed: {url}");
    }

    fn notify_closed(&self, _webview: WebView) {
        eprintln!("[servo] WebView closed by page (window.close)");
    }

    fn notify_crashed(&self, _webview: WebView, reason: String, backtrace: Option<String>) {
        eprintln!("[servo] CRASH: {reason}");
        if let Some(bt) = backtrace {
            eprintln!("{bt}");
        }
    }
}

// ── ApplicationHandler ────────────────────────────────────────────────────────

/// Height of the nav bar in logical pixels, measured empirically from the
/// Masonry flex_row layout (text_input + button with default theme padding).
/// Used to translate window coordinates into Servo viewport coordinates and
/// to size the Servo rendering surface. In a production app, query the
/// actual Masonry layout instead of using a constant.
const NAV_BAR_HEIGHT_LP: f64 = 72.0;

struct ServoXilemApp {
    masonry_state: MasonryState<'static>,
    app_driver: Box<dyn AppDriver>,
    servo_state: Option<ServoState>,
    /// Sends new CPU frames to the Xilem task_raw view.
    image_tx: watch::Sender<Option<ImageData>>,
    /// Navigation requests from the Xilem UI.
    nav_request: Arc<Mutex<Option<String>>>,
    /// URL to load on start.
    initial_url: Url,

    // ── Input tracking ───────────────────────────────────────────────────
    /// Last known cursor position in physical window coordinates.
    cursor_position: PhysicalPosition<f64>,
    /// Current keyboard modifier state (Ctrl, Shift, Alt, Super).
    modifiers: ModifiersState,
    /// Whether the cursor is currently inside the Servo viewport region.
    cursor_in_viewport: bool,
    /// Window scale factor (physical pixels per logical pixel).
    scale_factor: f64,
}

impl ApplicationHandler<MasonryUserEvent> for ServoXilemApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.masonry_state
            .handle_resumed(event_loop, &mut *self.app_driver);

        // Initialise Servo once the masonry window is up so we know the size.
        if self.servo_state.is_none() {
            let window_size = self
                .masonry_state
                .roots()
                .next()
                .map(|r| r.size())
                .unwrap_or(PhysicalSize::new(1280, 800));

            // Servo's viewport is the window minus the nav bar.
            let nav_h = self.nav_bar_height_px();
            let viewport = PhysicalSize::new(
                window_size.width,
                (window_size.height as f64 - nav_h).max(1.0) as u32,
            );
            match ServoState::init(viewport, self.initial_url.clone()) {
                Ok(state) => self.servo_state = Some(state),
                Err(err) => eprintln!("Servo init failed: {err}"),
            }
        }
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.masonry_state.handle_suspended(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        // ── Forward relevant events to Servo ─────────────────────────────
        //
        // We intercept winit events *before* Masonry processes them and
        // forward mouse, scroll, and keyboard events to Servo's WebView.
        // Masonry still receives every event for its own UI (nav bar, etc.);
        // there's no conflict because the Servo viewport is a passive image
        // widget that doesn't consume pointer events.

        if let WindowEvent::ModifiersChanged(mods) = &event {
            self.modifiers = mods.state();
        }

        if let WindowEvent::ScaleFactorChanged { scale_factor, .. } = &event {
            self.scale_factor = *scale_factor;
        }

        if let Some(ss) = &self.servo_state {
            let nav_h = self.nav_bar_height_px();
            match &event {
                // ── Resize ───────────────────────────────────────────────
                WindowEvent::Resized(new_size) => {
                    let viewport_h = (new_size.height as f64 - nav_h).max(1.0) as u32;
                    let viewport = PhysicalSize::new(new_size.width, viewport_h);
                    ss.render_ctx.resize(viewport);
                    ss.webview.resize(viewport);
                }

                // ── Cursor tracking ──────────────────────────────────────
                WindowEvent::CursorMoved { position, .. } => {
                    self.cursor_position = *position;
                    let was_in = self.cursor_in_viewport;
                    self.cursor_in_viewport = position.y >= nav_h;

                    if self.cursor_in_viewport {
                        let pt = self.servo_device_point(nav_h);
                        ss.webview
                            .notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(
                                pt.into(),
                            )));
                    } else if was_in {
                        // Cursor left the viewport → tell Servo.
                        ss.webview.notify_input_event(InputEvent::MouseLeftViewport(
                            MouseLeftViewportEvent::default(),
                        ));
                    }
                }
                WindowEvent::CursorLeft { .. } => {
                    if self.cursor_in_viewport {
                        ss.webview.notify_input_event(InputEvent::MouseLeftViewport(
                            MouseLeftViewportEvent::default(),
                        ));
                        self.cursor_in_viewport = false;
                    }
                }

                // ── Mouse buttons ────────────────────────────────────────
                WindowEvent::MouseInput { state, button, .. } if self.cursor_in_viewport => {
                    let action = match state {
                        ElementState::Pressed => MouseButtonAction::Down,
                        ElementState::Released => MouseButtonAction::Up,
                    };
                    let btn = match button {
                        winit::event::MouseButton::Left => ServoMouseButton::Left,
                        winit::event::MouseButton::Right => ServoMouseButton::Right,
                        winit::event::MouseButton::Middle => ServoMouseButton::Middle,
                        winit::event::MouseButton::Back => ServoMouseButton::Back,
                        winit::event::MouseButton::Forward => ServoMouseButton::Forward,
                        winit::event::MouseButton::Other(v) => ServoMouseButton::Other(*v),
                    };
                    let pt = self.servo_device_point(nav_h);
                    ss.webview
                        .notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                            action,
                            btn,
                            pt.into(),
                        )));
                }

                // ── Scroll wheel ─────────────────────────────────────────
                WindowEvent::MouseWheel { delta, .. } if self.cursor_in_viewport => {
                    let (dx, dy, mode) = match delta {
                        MouseScrollDelta::LineDelta(x, y) => {
                            ((*x as f64) * 38.0, (*y as f64) * 38.0, WheelMode::DeltaLine)
                        }
                        MouseScrollDelta::PixelDelta(d) => (d.x, d.y, WheelMode::DeltaPixel),
                    };
                    let pt = self.servo_device_point(nav_h);
                    ss.webview
                        .notify_input_event(InputEvent::Wheel(WheelEvent::new(
                            WheelDelta {
                                x: dx,
                                y: dy,
                                z: 0.0,
                                mode,
                            },
                            pt.into(),
                        )));
                }

                // ── Keyboard ─────────────────────────────────────────────
                //
                // Keyboard events are forwarded regardless of cursor position.
                // In a production app you would track focus (e.g. only forward
                // when the viewport image is focused, not the URL bar).
                WindowEvent::KeyboardInput {
                    event: key_event, ..
                } => {
                    let servo_event =
                        keyutils::keyboard_event_from_winit(key_event, self.modifiers);
                    ss.webview
                        .notify_input_event(InputEvent::Keyboard(servo_event));
                }

                _ => {}
            }
        }

        // ── Always forward to Masonry for its own UI ─────────────────────
        self.masonry_state
            .handle_window_event(event_loop, window_id, event, &mut *self.app_driver);
    }

    fn device_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        self.masonry_state
            .handle_device_event(event_loop, device_id, event, &mut *self.app_driver);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: MasonryUserEvent) {
        self.masonry_state
            .handle_user_event(event_loop, event, &mut *self.app_driver);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Note: `Resized` is handled in `window_event` above, including the
        // Servo viewport resize with the nav bar offset subtracted.
        event_loop.set_control_flow(ControlFlow::Poll);

        if let Some(ss) = &mut self.servo_state {
            // Handle any pending navigation from the URL bar.
            if let Some(raw_url) = self.nav_request.lock().unwrap().take() {
                // Auto-prefix https:// when no scheme is provided.
                let url =
                    Url::parse(&raw_url).or_else(|_| Url::parse(&format!("https://{raw_url}")));
                match url {
                    Ok(url) => ss.webview.load(url),
                    Err(_) => eprintln!("invalid URL: {raw_url}"),
                }
            }

            ss.servo.spin_event_loop();
            ss.webview.paint();

            // CPU readback from the swap chain's back buffer (previous frame, 1-frame delay).
            if let Some(rgba) = ss.render_ctx.read_full_frame() {
                let (width, height) = rgba.dimensions();
                let image_data = ImageData {
                    data: Blob::new(Arc::new(rgba.into_raw())),
                    format: ImageFormat::Rgba8,
                    alpha_type: ImageAlphaType::Alpha,
                    width,
                    height,
                };
                let _ = self.image_tx.send(Some(image_data));
            }
        }

        self.masonry_state.handle_about_to_wait(event_loop);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: winit::event::StartCause) {
        self.masonry_state.handle_new_events(event_loop, cause);
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.masonry_state.handle_exiting(event_loop);
    }

    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.masonry_state.handle_memory_warning(event_loop);
    }
}

impl ServoXilemApp {
    /// Convert the current cursor position to a Servo [`DevicePoint`],
    /// translating from window space to viewport space by subtracting the
    /// nav bar offset.
    fn servo_device_point(&self, nav_bar_px: f64) -> DevicePoint {
        DevicePoint::new(
            self.cursor_position.x as f32,
            (self.cursor_position.y - nav_bar_px) as f32,
        )
    }

    /// Nav bar height in physical pixels, derived from the logical-pixel
    /// constant and the current window scale factor.
    fn nav_bar_height_px(&self) -> f64 {
        NAV_BAR_HEIGHT_LP * self.scale_factor
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<(), EventLoopError> {
    aws_lc_rs::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let initial_url = resolve_initial_url().expect("failed to resolve initial URL");

    // Watch channel: about_to_wait sends frames; task_raw forwards them to AppState.
    let (image_tx, image_rx) = watch::channel(None::<ImageData>);
    // Shared slot: UI writes a URL here on "Go" click; about_to_wait reads it.
    let nav_request: Arc<Mutex<Option<String>>> = Default::default();

    let app_state = AppState {
        nav_input: initial_url.to_string(),
        current_image: None,
    };

    let window_options =
        WindowOptions::new("servo-xilem demo").with_min_inner_size(LogicalSize::new(800.0, 600.0));

    // Each rebuild gets a fresh receiver clone; all clones share the same channel.
    // task_raw uses only the first clone (it runs once), which is fine.
    let nav_request_for_view = nav_request.clone();
    let xilem = Xilem::new_simple(
        app_state,
        move |state: &mut AppState| {
            let rx = image_rx.clone();
            let nr = nav_request_for_view.clone();
            app_logic(state, rx, nr)
        },
        window_options,
    );

    let event_loop = EventLoop::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    let (driver, windows) =
        xilem.into_driver_and_windows(move |event| proxy.send_event(event).map_err(|e| e.0));

    let masonry_state =
        MasonryState::new(event_loop.create_proxy(), windows, default_property_set());

    let mut app = ServoXilemApp {
        masonry_state,
        app_driver: Box::new(driver),
        servo_state: None,
        image_tx,
        nav_request,
        initial_url,
        cursor_position: PhysicalPosition::new(0.0, 0.0),
        modifiers: ModifiersState::default(),
        cursor_in_viewport: false,
        scale_factor: 1.0,
    };

    event_loop.run_app(&mut app)
}

// ── URL resolution ────────────────────────────────────────────────────────────

fn resolve_initial_url() -> Result<Url, String> {
    if let Some(arg) = std::env::args().nth(1) {
        if let Ok(url) = Url::parse(&arg) {
            return Ok(url);
        }
        let path = std::path::Path::new(&arg);
        let abs = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(path)
        };
        return Url::from_file_path(&abs)
            .map_err(|_| format!("not a valid URL or file path: {arg}"));
    }

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("animated.html");
    Url::from_file_path(&fixture).map_err(|_| format!("fixture not found: {}", fixture.display()))
}
