//! Demo embedding Servo in a GPUI 0.2 application.
//!
//! Servo renders offscreen via surfman/GL. Each frame, pixels are read back to
//! CPU via `read_full_frame()`, converted from RGBA to BGRA (GPUI's internal
//! format), wrapped in a `RenderImage`, and displayed via `img(ImageSource::Render(...))`.
//!
//! The GPUI window provides a URL bar above the Servo viewport. Mouse, scroll,
//! and keyboard events are forwarded to Servo so pages are interactive.
//!
//! ## Rendering loop
//!
//! GPUI drives rendering via `window.request_animation_frame()`. Each call to
//! `render()` polls Servo, reads back the latest frame, and then schedules the
//! next animation frame — giving a continuous ~vsync-rate render loop without
//! a background thread (Servo is `!Send`).
//!
//! Usage:
//!   cargo run -p demo-servo-gpui -- https://example.com
//!   cargo run -p demo-servo-gpui -- servo.org        # auto-prefixes https://
//!   cargo run -p demo-servo-gpui                     # opens built-in fixture page

mod keyutils;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use euclid::Scale;
use gpui::{
    App, Application, Bounds, Context, FocusHandle, ImageSource, InteractiveElement,
    IntoElement, KeyDownEvent, KeyUpEvent, MouseButton as GpuiMouseButton, MouseDownEvent,
    MouseMoveEvent as GpuiMouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement, Pixels,
    Point, Render, RenderImage, ScrollDelta, ScrollWheelEvent, Styled, Window, WindowBounds,
    WindowOptions, div, img, prelude::*, px, rgb, size,
};
use image::{Frame, ImageBuffer, Rgba};
use rustls::crypto::aws_lc_rs;
use servo::{
    DevicePoint, EventLoopWaker, InputEvent,
    MouseButton as ServoMouseButton, MouseButtonAction, MouseButtonEvent,
    MouseLeftViewportEvent, MouseMoveEvent as ServoMouseMoveEvent, Servo, ServoBuilder,
    WebView, WebViewBuilder, WebViewDelegate, WheelDelta, WheelEvent, WheelMode,
};
use servo_wgpu_interop_adapter::ServoWgpuRenderingContext;
use smallvec::SmallVec;
use url::Url;
use winit::dpi::PhysicalSize;

// ── Constants ────────────────────────────────────────────────────────────────

/// Height of the URL navigation bar in logical pixels.
const NAV_BAR_HEIGHT: f32 = 48.0;

const DEFAULT_WIDTH: f32 = 1280.0;
const DEFAULT_HEIGHT: f32 = 800.0;

// ── App state ────────────────────────────────────────────────────────────────

struct ServoView {
    // Servo (main-thread only, !Send)
    servo: Servo,
    webview: WebView,
    render_ctx: Rc<ServoWgpuRenderingContext>,

    // Latest rendered frame as BGRA RenderImage
    frame: Option<Arc<RenderImage>>,

    // URL bar state
    url_text: String,
    url_focused: bool,
    url_focus: FocusHandle,

    // Viewport input tracking
    viewport_focus: FocusHandle,
    cursor_pos: Point<Pixels>,
    cursor_in_viewport: bool,
}

impl ServoView {
    fn new(initial_url: Url, cx: &mut Context<Self>) -> Self {
        let render_ctx = Rc::new(
            ServoWgpuRenderingContext::new(PhysicalSize::new(
                DEFAULT_WIDTH as u32,
                (DEFAULT_HEIGHT - NAV_BAR_HEIGHT) as u32,
            ))
            .expect("failed to create ServoWgpuRenderingContext"),
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

        Self {
            servo,
            webview,
            render_ctx,
            frame: None,
            url_text: initial_url.to_string(),
            url_focused: false,
            url_focus: cx.focus_handle(),
            viewport_focus: cx.focus_handle(),
            cursor_pos: Point::default(),
            cursor_in_viewport: false,
        }
    }

    /// Navigate to the URL currently in the URL bar.
    fn navigate(&mut self) {
        let raw = &self.url_text;
        let url = Url::parse(raw).or_else(|_| Url::parse(&format!("https://{raw}")));
        match url {
            Ok(url) => {
                eprintln!("[demo] navigating to: {url}");
                self.webview.load(url);
            }
            Err(_) => eprintln!("[demo] invalid URL: {raw}"),
        }
        self.url_focused = false;
    }

    /// Convert a GPUI window-space point to a Servo DevicePoint by subtracting
    /// the nav-bar offset.
    fn servo_point(&self, pos: Point<Pixels>) -> DevicePoint {
        DevicePoint::new(
            f32::from(pos.x),
            (f32::from(pos.y) - NAV_BAR_HEIGHT).max(0.0),
        )
    }

    /// Forward a mouse-down event to Servo.
    fn servo_mouse_down(&mut self, button: ServoMouseButton, pos: Point<Pixels>) {
        let pt = self.servo_point(pos);
        self.webview.notify_input_event(InputEvent::MouseButton(
            MouseButtonEvent::new(
                MouseButtonAction::Down,
                button,
                servo::WebViewPoint::Device(pt),
            ),
        ));
    }

    /// Forward a mouse-up event to Servo.
    fn servo_mouse_up(&mut self, button: ServoMouseButton, pos: Point<Pixels>) {
        let pt = self.servo_point(pos);
        self.webview.notify_input_event(InputEvent::MouseButton(
            MouseButtonEvent::new(
                MouseButtonAction::Up,
                button,
                servo::WebViewPoint::Device(pt),
            ),
        ));
    }
}

// ── Render ───────────────────────────────────────────────────────────────────

impl Render for ServoView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // ── Poll Servo and read the latest frame ─────────────────────────
        self.servo.spin_event_loop();
        self.webview.paint();

        if let Some(rgba) = self.render_ctx.read_full_frame() {
            let (w, h) = rgba.dimensions();
            let mut raw = rgba.into_raw();
            // Servo outputs RGBA; GPUI expects BGRA — swap channels 0 and 2.
            for px in raw.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            let bgra: ImageBuffer<Rgba<u8>, Vec<u8>> =
                ImageBuffer::from_raw(w, h, raw).expect("invalid dimensions");
            let render_image = RenderImage::new(SmallVec::from_elem(Frame::new(bgra), 1));
            self.frame = Some(Arc::new(render_image));
        }

        // Keep re-rendering every frame for continuous Servo polling.
        window.request_animation_frame();

        // ── Build UI ─────────────────────────────────────────────────────

        // URL bar
        let url_text = self.url_text.clone();
        let url_focused = self.url_focused;
        let url_bar = div()
            .h(px(NAV_BAR_HEIGHT))
            .w_full()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_3()
            .bg(rgb(0x2d2d2d))
            .border_b_1()
            .border_color(rgb(0x444444))
            .child(
                div()
                    .flex_1()
                    .h(px(28.0))
                    .flex()
                    .items_center()
                    .px_2()
                    .rounded_md()
                    .bg(if url_focused { rgb(0x3a3a3a) } else { rgb(0x252525) })
                    .border_1()
                    .border_color(if url_focused { rgb(0x5b9bd5) } else { rgb(0x444444) })
                    .text_color(rgb(0xe0e0e0))
                    .text_sm()
                    .track_focus(&self.url_focus)
                    .on_mouse_down(
                        GpuiMouseButton::Left,
                        cx.listener(|view, _event: &MouseDownEvent, window, _cx| {
                            view.url_focused = true;
                            view.url_focus.focus(window);
                        }),
                    )
                    .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                        handle_url_key(view, event, window, cx);
                    }))
                    .child(url_text),
            );

        // Servo viewport
        let viewport: gpui::AnyElement = if let Some(frame) = &self.frame {
            let source = ImageSource::Render(frame.clone());
            div()
                .flex_1()
                .w_full()
                .overflow_hidden()
                .track_focus(&self.viewport_focus)
                .on_mouse_down(
                    GpuiMouseButton::Left,
                    cx.listener(|view, event: &MouseDownEvent, window, _cx| {
                        view.url_focused = false;
                        view.viewport_focus.focus(window);
                        view.servo_mouse_down(ServoMouseButton::Left, event.position);
                    }),
                )
                .on_mouse_down(
                    GpuiMouseButton::Right,
                    cx.listener(|view, event: &MouseDownEvent, _window, _cx| {
                        view.servo_mouse_down(ServoMouseButton::Right, event.position);
                    }),
                )
                .on_mouse_down(
                    GpuiMouseButton::Middle,
                    cx.listener(|view, event: &MouseDownEvent, _window, _cx| {
                        view.servo_mouse_down(ServoMouseButton::Middle, event.position);
                    }),
                )
                .on_mouse_up(
                    GpuiMouseButton::Left,
                    cx.listener(|view, event: &MouseUpEvent, _window, _cx| {
                        view.servo_mouse_up(ServoMouseButton::Left, event.position);
                    }),
                )
                .on_mouse_up(
                    GpuiMouseButton::Right,
                    cx.listener(|view, event: &MouseUpEvent, _window, _cx| {
                        view.servo_mouse_up(ServoMouseButton::Right, event.position);
                    }),
                )
                .on_mouse_up(
                    GpuiMouseButton::Middle,
                    cx.listener(|view, event: &MouseUpEvent, _window, _cx| {
                        view.servo_mouse_up(ServoMouseButton::Middle, event.position);
                    }),
                )
                .on_mouse_move(cx.listener(|view, event: &GpuiMouseMoveEvent, _window, _cx| {
                    let pos = event.position;
                    view.cursor_pos = pos;
                    let was_in = view.cursor_in_viewport;
                    view.cursor_in_viewport = f32::from(pos.y) >= NAV_BAR_HEIGHT;

                    if view.cursor_in_viewport {
                        let pt = view.servo_point(pos);
                        view.webview.notify_input_event(InputEvent::MouseMove(
                            ServoMouseMoveEvent::new(servo::WebViewPoint::Device(pt)),
                        ));
                    } else if was_in {
                        view.webview.notify_input_event(InputEvent::MouseLeftViewport(
                            MouseLeftViewportEvent::default(),
                        ));
                    }
                }))
                .on_scroll_wheel(cx.listener(|view, event: &ScrollWheelEvent, _window, _cx| {
                    let pos = event.position;
                    let (dx, dy, mode) = match event.delta {
                        ScrollDelta::Pixels(p) => {
                            (f32::from(p.x) as f64, f32::from(p.y) as f64, WheelMode::DeltaPixel)
                        }
                        ScrollDelta::Lines(p) => {
                            (p.x as f64 * 38.0, p.y as f64 * 38.0, WheelMode::DeltaLine)
                        }
                    };
                    let pt = view.servo_point(pos);
                    view.webview.notify_input_event(InputEvent::Wheel(WheelEvent::new(
                        WheelDelta { x: dx, y: dy, z: 0.0, mode },
                        servo::WebViewPoint::Device(pt),
                    )));
                }))
                .on_key_down(cx.listener(|view, event: &KeyDownEvent, _window, _cx| {
                    let kbd = keyutils::keyboard_event_from_gpui_down(event);
                    view.webview.notify_input_event(InputEvent::Keyboard(kbd));
                }))
                .on_key_up(cx.listener(|view, event: &KeyUpEvent, _window, _cx| {
                    let kbd = keyutils::keyboard_event_from_gpui_up(event);
                    view.webview.notify_input_event(InputEvent::Keyboard(kbd));
                }))
                .child(
                    img(source)
                        .size_full()
                        .object_fit(ObjectFit::Fill),
                )
                .into_any()
        } else {
            div()
                .flex_1()
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(0xaaaaaa))
                .child("Servo loading…")
                .into_any()
        };

        // Root container captures resize and mouse-leave
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e1e))
            .on_mouse_move(cx.listener(|view, event: &GpuiMouseMoveEvent, _window, _cx| {
                // Track cursor entering/leaving viewport area (for MouseLeftViewport)
                let in_vp = f32::from(event.position.y) >= NAV_BAR_HEIGHT;
                if !in_vp && view.cursor_in_viewport {
                    view.webview.notify_input_event(InputEvent::MouseLeftViewport(
                        MouseLeftViewportEvent::default(),
                    ));
                    view.cursor_in_viewport = false;
                }
            }))
            .child(url_bar)
            .child(viewport)
    }
}

// ── URL bar key handler ───────────────────────────────────────────────────────

fn handle_url_key(
    view: &mut ServoView,
    event: &KeyDownEvent,
    _window: &mut Window,
    _cx: &mut Context<ServoView>,
) {
    let key = &event.keystroke.key;
    match key.as_str() {
        "enter" | "return" | "Enter" => {
            view.navigate();
        }
        "escape" | "Escape" => {
            view.url_focused = false;
        }
        "backspace" | "Backspace" => {
            view.url_text.pop();
        }
        _ => {
            // Append printable characters (no ctrl/alt modifiers).
            if !event.keystroke.modifiers.control && !event.keystroke.modifiers.alt {
                if let Some(c) = &event.keystroke.key_char {
                    if !c.is_empty() {
                        view.url_text.push_str(c);
                    }
                }
            }
        }
    }
}

// ── Servo support ─────────────────────────────────────────────────────────────

/// A no-op waker — GPUI polls on every animation frame, so no explicit waking needed.
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

// ── URL resolution ────────────────────────────────────────────────────────────

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

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    aws_lc_rs::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let initial_url = resolve_initial_url().expect("failed to resolve initial URL");

    Application::new().run(move |cx: &mut App| {
        let initial_url = initial_url.clone();
        let bounds = Bounds::centered(
            None,
            size(px(DEFAULT_WIDTH), px(DEFAULT_HEIGHT)),
            cx,
        );
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("demo-servo-gpui".into()),
                    ..Default::default()
                }),
                focus: true,
                ..Default::default()
            },
            move |_window, cx| cx.new(|cx| ServoView::new(initial_url.clone(), cx)),
        )
        .expect("failed to open window");
        cx.activate(true);
    });
}
