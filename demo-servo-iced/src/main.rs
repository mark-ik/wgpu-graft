//! Demo embedding Servo in an iced 0.14 application.
//!
//! Servo renders offscreen via surfman/GL. Each frame, pixels are read back to
//! CPU via `read_full_frame()` and displayed as an `iced::widget::image`.
//!
//! The iced UI provides a URL bar above the Servo viewport. Mouse, scroll,
//! and keyboard events are forwarded to Servo so pages are interactive.
//!
//! ## Frame upload strategy
//!
//! Iced's wgpu renderer uploads images >2MB asynchronously to the GPU atlas.
//! A 1280×750 RGBA frame is ~3.8MB, so naively creating a new `Handle` each
//! tick would cause flicker: the async upload never finishes before the Handle
//! changes. We solve this by calling `iced::widget::image::allocate()` which
//! pre-allocates the GPU texture and guarantees it is ready for the next frame.
//! A new frame is only read after the previous allocation completes.
//!
//! Usage:
//!   cargo run -p demo-servo-iced -- https://example.com
//!   cargo run -p demo-servo-iced -- servo.org        # auto-prefixes https://
//!   cargo run -p demo-servo-iced                     # opens built-in fixture page

mod keyutils;

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use euclid::Scale;
use iced::widget::{column, image, text, text_input};
use iced::{event, keyboard, mouse, window, Element, Event, Length, Size, Subscription, Task};
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

/// Estimated height of the URL bar in logical pixels. Used to translate
/// window coordinates into Servo viewport coordinates and to size the
/// Servo rendering surface. In a production app you'd query the actual
/// layout instead of using a constant.
const NAV_BAR_HEIGHT: f32 = 50.0;

/// Default viewport size (logical pixels) used before the first resize.
const DEFAULT_WIDTH: f32 = 1280.0;
const DEFAULT_HEIGHT: f32 = 800.0;

// ── App state ────────────────────────────────────────────────────────────────

struct AppState {
    // Servo
    servo: Servo,
    webview: WebView,
    render_ctx: Rc<ServoWgpuRenderingContext>,

    // UI
    url_input: String,
    frame: Option<image::Handle>,
    viewport_size: Size,

    /// True while an `allocate()` Task is in-flight. Gates new frame reads
    /// so we don't flood iced's image cache with un-uploaded textures.
    allocating: bool,
    /// Holds the current GPU allocation so it isn't freed until the next
    /// frame replaces it.
    _allocation: Option<image::Allocation>,

    // Input tracking
    cursor_position: iced::Point,
    cursor_in_viewport: bool,
}

// ── Messages ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Message {
    Tick,
    UrlInputChanged(String),
    Navigate,
    IcedEvent(Event),
    FrameAllocated(Result<image::Allocation, image::Error>),
}

// ── Boot ─────────────────────────────────────────────────────────────────────

fn boot() -> (AppState, Task<Message>) {
    let initial_url = resolve_initial_url().expect("failed to resolve initial URL");

    let viewport_w = DEFAULT_WIDTH;
    let viewport_h = DEFAULT_HEIGHT - NAV_BAR_HEIGHT;

    let render_ctx = Rc::new(
        ServoWgpuRenderingContext::new(PhysicalSize::new(viewport_w as u32, viewport_h as u32))
            .expect("failed to create rendering context"),
    );

    let servo = ServoBuilder::default()
        .event_loop_waker(Box::new(NoopWaker))
        .build();
    servo.setup_logging();

    let delegate = Rc::new(DemoDelegate);

    let webview = WebViewBuilder::new(&servo, render_ctx.clone())
        .url(initial_url.clone())
        .hidpi_scale_factor(Scale::new(1.0))
        .delegate(delegate)
        .build();

    let state = AppState {
        servo,
        webview,
        render_ctx,
        url_input: initial_url.to_string(),
        frame: None,
        viewport_size: Size::new(viewport_w, viewport_h),
        allocating: false,
        _allocation: None,
        cursor_position: iced::Point::ORIGIN,
        cursor_in_viewport: false,
    };

    (state, Task::none())
}

// ── Update ───────────────────────────────────────────────────────────────────

fn update(state: &mut AppState, message: Message) -> Task<Message> {
    match message {
        Message::Tick => {
            state.servo.spin_event_loop();
            state.webview.paint();

            // Only read a new frame when the previous one has been fully
            // allocated on the GPU. This prevents flooding iced's image
            // cache with >2MB textures that never finish async upload
            // before the Handle changes (which causes flicker).
            if !state.allocating {
                if let Some(rgba) = state.render_ctx.read_full_frame() {
                    let (w, h) = rgba.dimensions();
                    let handle = image::Handle::from_rgba(w, h, rgba.into_raw());
                    state.allocating = true;
                    return iced::widget::image::allocate(handle)
                        .map(Message::FrameAllocated);
                }
            }
        }

        Message::FrameAllocated(result) => {
            state.allocating = false;
            if let Ok(allocation) = result {
                state.frame = Some(allocation.handle().clone());
                state._allocation = Some(allocation);
            }
        }

        Message::UrlInputChanged(url) => {
            state.url_input = url;
        }

        Message::Navigate => {
            let raw = &state.url_input;
            let url = Url::parse(raw)
                .or_else(|_| Url::parse(&format!("https://{raw}")));
            match url {
                Ok(url) => state.webview.load(url),
                Err(_) => eprintln!("invalid URL: {raw}"),
            }
        }

        Message::IcedEvent(event) => {
            handle_event(state, event);
        }
    }

    Task::none()
}

fn handle_event(state: &mut AppState, event: Event) {
    match event {
        // ── Window resize ───────────────────────────────────────────────
        Event::Window(window::Event::Resized(new_size)) => {
            let vp_w = new_size.width;
            let vp_h = (new_size.height - NAV_BAR_HEIGHT).max(1.0);
            state.viewport_size = Size::new(vp_w, vp_h);

            let physical = PhysicalSize::new(vp_w as u32, vp_h as u32);
            state.render_ctx.resize_viewport(physical);
            state.webview.resize(physical);
        }

        // ── Cursor tracking ─────────────────────────────────────────────
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            state.cursor_position = position;
            let was_in = state.cursor_in_viewport;
            state.cursor_in_viewport = position.y >= NAV_BAR_HEIGHT;

            if state.cursor_in_viewport {
                let pt = servo_point(position);
                state.webview.notify_input_event(InputEvent::MouseMove(
                    MouseMoveEvent::new(servo::WebViewPoint::Device(pt)),
                ));
            } else if was_in {
                state.webview.notify_input_event(InputEvent::MouseLeftViewport(
                    MouseLeftViewportEvent::default(),
                ));
            }
        }

        Event::Mouse(mouse::Event::CursorLeft) => {
            if state.cursor_in_viewport {
                state.webview.notify_input_event(InputEvent::MouseLeftViewport(
                    MouseLeftViewportEvent::default(),
                ));
                state.cursor_in_viewport = false;
            }
        }

        // ── Mouse buttons ───────────────────────────────────────────────
        Event::Mouse(mouse::Event::ButtonPressed(btn)) if state.cursor_in_viewport => {
            if let Some(servo_btn) = map_mouse_button(btn) {
                let pt = servo_point(state.cursor_position);
                state.webview.notify_input_event(InputEvent::MouseButton(
                    MouseButtonEvent::new(
                        MouseButtonAction::Down,
                        servo_btn,
                        servo::WebViewPoint::Device(pt),
                    ),
                ));
            }
        }

        Event::Mouse(mouse::Event::ButtonReleased(btn)) if state.cursor_in_viewport => {
            if let Some(servo_btn) = map_mouse_button(btn) {
                let pt = servo_point(state.cursor_position);
                state.webview.notify_input_event(InputEvent::MouseButton(
                    MouseButtonEvent::new(
                        MouseButtonAction::Up,
                        servo_btn,
                        servo::WebViewPoint::Device(pt),
                    ),
                ));
            }
        }

        // ── Scroll wheel ────────────────────────────────────────────────
        Event::Mouse(mouse::Event::WheelScrolled { delta }) if state.cursor_in_viewport => {
            let (dx, dy, mode) = match delta {
                mouse::ScrollDelta::Lines { x, y } => {
                    ((x as f64) * 38.0, (y as f64) * 38.0, WheelMode::DeltaLine)
                }
                mouse::ScrollDelta::Pixels { x, y } => {
                    (x as f64, y as f64, WheelMode::DeltaPixel)
                }
            };
            let pt = servo_point(state.cursor_position);
            state.webview.notify_input_event(InputEvent::Wheel(
                WheelEvent::new(
                    WheelDelta { x: dx, y: dy, z: 0.0, mode },
                    servo::WebViewPoint::Device(pt),
                ),
            ));
        }

        // ── Keyboard ────────────────────────────────────────────────────
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

    let content: Element<Message> = if let Some(handle) = &state.frame {
        image(handle)
            .content_fit(iced::ContentFit::Fill)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    } else {
        text("Servo loading…")
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    };

    column![url_bar, content].into()
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

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Convert an iced Point (logical window coords) to a Servo DevicePoint
/// by subtracting the nav bar height.
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

/// A no-op waker — iced polls on a timer, so we don't need explicit waking.
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

// ── URL resolution ───────────────────────────────────────────────────────────

fn resolve_initial_url() -> Result<Url, String> {
    if let Some(arg) = std::env::args().nth(1) {
        return resolve_url_argument(&arg);
    }

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("animated.html");
    Url::from_file_path(&fixture)
        .map_err(|_| format!("fixture not found: {}", fixture.display()))
}

fn resolve_url_argument(argument: &str) -> Result<Url, String> {
    if let Ok(url) = Url::parse(argument) {
        return Ok(url);
    }
    if let Ok(url) = Url::parse(&format!("https://{argument}")) {
        return Ok(url);
    }
    let candidate = PathBuf::from(argument);
    let absolute = if candidate.is_absolute() {
        candidate
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(candidate)
    };
    Url::from_file_path(&absolute)
        .map_err(|_| format!("not a valid URL or file path: {argument}"))
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() -> iced::Result {
    aws_lc_rs::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    iced::application(boot, update, view)
        .title("demo-servo-iced")
        .subscription(subscription)
        .window_size((DEFAULT_WIDTH, DEFAULT_HEIGHT))
        .run()
}
