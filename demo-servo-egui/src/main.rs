//! Demo embedding Servo in an egui/eframe application, zero-copy.
//!
//! Servo renders offscreen via surfman/GL. Each frame its GL output is imported
//! (via `grafting` / `servo-wgpu-interop-adapter`) directly into a
//! `wgpu::Texture` on eframe's own wgpu device, then handed to egui's renderer
//! as a native texture and drawn as an `egui::Image`. No CPU readback.
//!
//! Build with `--features cpu-readback` to use the CPU readback path instead
//! (an alternative mechanism; zero-copy is the default and the point).
//!
//! The egui UI provides a URL bar above the Servo viewport. Mouse, scroll, and
//! keyboard events over the viewport are forwarded to Servo so pages are
//! interactive (links, scrolling, text input).
//!
//! Usage:
//!   cargo run -p demo-servo-egui -- https://example.com
//!   cargo run -p demo-servo-egui -- servo.org        # auto-prefixes https://
//!   cargo run -p demo-servo-egui                     # opens built-in fixture

mod keyutils;

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use demo_support::{DemoStatus, RenderPath};
use eframe::egui_wgpu;
use eframe::wgpu;
use eframe::{App, CreationContext, Frame, NativeOptions, egui};
use euclid::Scale;
use rustls::crypto::aws_lc_rs;
use servo::{
    DevicePoint, EventLoopWaker, InputEvent, MouseButton as ServoMouseButton, MouseButtonAction,
    MouseButtonEvent, MouseLeftViewportEvent, MouseMoveEvent, Servo, ServoBuilder, WebView,
    WebViewBuilder, WebViewDelegate, WebViewPoint, WheelDelta, WheelEvent, WheelMode,
};
use servo_wgpu_interop_adapter::ServoWgpuInteropAdapter;
use url::Url;
use winit::dpi::PhysicalSize;

const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 800;

// ── App ────────────────────────────────────────────────────────────────────

struct ServoEguiApp {
    // egui's wgpu render state: its device, queue, and the renderer we register
    // the imported Servo texture with. Only needed for the zero-copy path.
    #[cfg(not(feature = "cpu-readback"))]
    render_state: egui_wgpu::RenderState,

    // Servo + the interop adapter, built on egui's device.
    servo: Servo,
    webview: WebView,
    adapter: ServoWgpuInteropAdapter,
    /// Set by the webview delegate when Servo has a fresh frame. We only
    /// paint+import when this is set, so we never read a blank/mid-swap buffer
    /// (the cause of flicker when importing on every egui repaint).
    frame_ready: Arc<AtomicBool>,

    // Zero-copy path: a stable egui TextureId rebound to each imported frame,
    // and the live texture kept alive until egui's render pass samples it.
    #[cfg(not(feature = "cpu-readback"))]
    texture_id: Option<egui::TextureId>,
    #[cfg(not(feature = "cpu-readback"))]
    current_texture: Option<wgpu::Texture>,
    // CPU readback path: an egui-managed texture uploaded from the read-back image.
    #[cfg(feature = "cpu-readback")]
    cpu_texture: Option<egui::TextureHandle>,

    url_input: String,
    url_focused: bool,
    status: DemoStatus,

    viewport_size: PhysicalSize<u32>,
    /// The egui scale factor (device px per logical point) last given to Servo.
    current_ppp: f32,
    image_rect: egui::Rect,
    cursor_in_viewport: bool,
    last_pointer: egui::Pos2,
}

impl ServoEguiApp {
    fn new(cc: &CreationContext<'_>, initial_url: Url) -> Self {
        let render_state = cc
            .wgpu_render_state
            .as_ref()
            .expect("demo-servo-egui requires the wgpu renderer (eframe Renderer::Wgpu)")
            .clone();

        // Match Servo's layout scale to egui's actual device-pixel ratio, like
        // the winit demo does. With a hardcoded 1.0 on a HiDPI display Servo
        // lays out at 1x into a physical-sized surface, filling one quadrant.
        let ppp = cc.egui_ctx.pixels_per_point();
        let initial = PhysicalSize::new(
            (DEFAULT_WIDTH as f32 * ppp) as u32,
            (DEFAULT_HEIGHT as f32 * ppp) as u32,
        );
        let adapter = ServoWgpuInteropAdapter::new(
            render_state.device.clone(),
            render_state.queue.clone(),
            initial,
        )
        .expect("failed to create Servo interop adapter on egui's wgpu device");

        let servo = ServoBuilder::default()
            .event_loop_waker(Box::new(NoopWaker))
            .build();
        servo.setup_logging();

        let frame_ready = Arc::new(AtomicBool::new(true));
        let webview = WebViewBuilder::new(&servo, adapter.rendering_context())
            .url(initial_url.clone())
            .hidpi_scale_factor(Scale::new(ppp))
            .delegate(Rc::new(DemoDelegate {
                frame_ready: frame_ready.clone(),
                egui_ctx: cc.egui_ctx.clone(),
            }))
            .build();

        let path = if cfg!(feature = "cpu-readback") {
            RenderPath::CpuReadback
        } else {
            RenderPath::GpuImport
        };

        Self {
            #[cfg(not(feature = "cpu-readback"))]
            render_state,
            servo,
            webview,
            adapter,
            frame_ready,
            #[cfg(not(feature = "cpu-readback"))]
            texture_id: None,
            #[cfg(not(feature = "cpu-readback"))]
            current_texture: None,
            #[cfg(feature = "cpu-readback")]
            cpu_texture: None,
            url_input: initial_url.to_string(),
            url_focused: false,
            status: DemoStatus::new(path),
            viewport_size: initial,
            current_ppp: ppp,
            image_rect: egui::Rect::NOTHING,
            cursor_in_viewport: false,
            last_pointer: egui::Pos2::ZERO,
        }
    }

    fn navigate(&mut self) {
        let raw = self.url_input.trim().to_string();
        match Url::parse(&raw).or_else(|_| Url::parse(&format!("https://{raw}"))) {
            Ok(url) => self.webview.load(url),
            Err(_) => eprintln!("invalid URL: {raw}"),
        }
    }

    /// Convert an egui screen position (points) to a Servo device point
    /// relative to the viewport image, or `None` if outside it.
    fn servo_point(&self, pos: egui::Pos2, pixels_per_point: f32) -> Option<DevicePoint> {
        if !self.image_rect.contains(pos) {
            return None;
        }
        let local = pos - self.image_rect.min;
        Some(DevicePoint::new(
            local.x * pixels_per_point,
            local.y * pixels_per_point,
        ))
    }

    /// Forward this frame's pointer/scroll/keyboard events to Servo. Keyboard
    /// events are suppressed while the URL bar has focus so typing a URL does
    /// not also type into the page.
    fn forward_input(&mut self, ctx: &egui::Context) {
        if self.image_rect == egui::Rect::NOTHING {
            return;
        }
        let ppp = ctx.pixels_per_point();
        let (events, modifiers) = ctx.input(|i| (i.events.clone(), i.modifiers));

        for event in events {
            match event {
                egui::Event::PointerMoved(pos) => {
                    self.last_pointer = pos;
                    if let Some(pt) = self.servo_point(pos, ppp) {
                        self.cursor_in_viewport = true;
                        self.webview
                            .notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(
                                WebViewPoint::Device(pt),
                            )));
                    } else if self.cursor_in_viewport {
                        self.cursor_in_viewport = false;
                        self.webview.notify_input_event(InputEvent::MouseLeftViewport(
                            MouseLeftViewportEvent::default(),
                        ));
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } => {
                    if let (Some(btn), Some(pt)) = (map_button(button), self.servo_point(pos, ppp)) {
                        let action = if pressed {
                            MouseButtonAction::Down
                        } else {
                            MouseButtonAction::Up
                        };
                        self.webview
                            .notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                                action,
                                btn,
                                WebViewPoint::Device(pt),
                            )));
                    }
                }
                egui::Event::MouseWheel { unit, delta, .. } => {
                    if let Some(pt) = self.servo_point(self.last_pointer, ppp) {
                        let (dx, dy, mode) = match unit {
                            egui::MouseWheelUnit::Line => {
                                (delta.x as f64 * 38.0, delta.y as f64 * 38.0, WheelMode::DeltaLine)
                            }
                            egui::MouseWheelUnit::Point => {
                                (delta.x as f64, delta.y as f64, WheelMode::DeltaPixel)
                            }
                            egui::MouseWheelUnit::Page => (
                                delta.x as f64 * 380.0,
                                delta.y as f64 * 380.0,
                                WheelMode::DeltaLine,
                            ),
                        };
                        self.webview
                            .notify_input_event(InputEvent::Wheel(WheelEvent::new(
                                WheelDelta {
                                    x: dx,
                                    y: dy,
                                    z: 0.0,
                                    mode,
                                },
                                WebViewPoint::Device(pt),
                            )));
                    }
                }
                egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    modifiers: key_mods,
                    ..
                } => {
                    if !self.url_focused {
                        if let Some(kbd) = keyutils::named_key_event(key, pressed, key_mods, repeat) {
                            self.webview.notify_input_event(InputEvent::Keyboard(kbd));
                        }
                    }
                }
                egui::Event::Text(text) => {
                    if !self.url_focused {
                        for kbd in keyutils::text_key_events(&text, modifiers) {
                            self.webview.notify_input_event(InputEvent::Keyboard(kbd));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Resize the Servo viewport to match the central-panel image rect, and
    /// keep Servo's HiDPI scale in sync with egui's. A dead-band avoids
    /// resizing on sub-pixel rect jitter, which would blank the surface every
    /// frame (flicker).
    fn sync_viewport(&mut self, ctx: &egui::Context) {
        if self.image_rect == egui::Rect::NOTHING {
            return;
        }
        let ppp = ctx.pixels_per_point();
        if (ppp - self.current_ppp).abs() > f32::EPSILON {
            self.current_ppp = ppp;
            self.webview.set_hidpi_scale_factor(Scale::new(ppp));
        }
        let w = (self.image_rect.width() * ppp).round().max(1.0) as u32;
        let h = (self.image_rect.height() * ppp).round().max(1.0) as u32;
        let dw = (w as i64 - self.viewport_size.width as i64).abs();
        let dh = (h as i64 - self.viewport_size.height as i64).abs();
        if dw > 2 || dh > 2 {
            self.viewport_size = PhysicalSize::new(w, h);
            self.adapter
                .rendering_context_handle()
                .resize_viewport(self.viewport_size);
            self.webview.resize(self.viewport_size);
        }
    }

    /// Zero-copy: import Servo's GL frame into a wgpu texture on egui's device
    /// and (re)bind it to our stable egui TextureId.
    #[cfg(not(feature = "cpu-readback"))]
    fn acquire_frame(&mut self, _ctx: &egui::Context) {
        let imported = match self.adapter.import_current_frame_default() {
            Ok(imported) => imported,
            Err(e) => {
                self.status.set_fallback_error(&e);
                return;
            }
        };

        let device = &self.render_state.device;
        let new_id = {
            let view = imported
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut renderer = self.render_state.renderer.write();
            match self.texture_id {
                Some(id) => {
                    renderer.update_egui_texture_from_wgpu_texture(
                        device,
                        &view,
                        wgpu::FilterMode::Linear,
                        id,
                    );
                    None
                }
                None => Some(renderer.register_native_texture(
                    device,
                    &view,
                    wgpu::FilterMode::Linear,
                )),
            }
        };
        if let Some(id) = new_id {
            self.texture_id = Some(id);
        }

        self.status
            .set_frame(RenderPath::GpuImport, imported.size.width, imported.size.height);
        self.current_texture = Some(imported.texture);
    }

    /// CPU readback alternative: read the GL frame to a CPU image and upload it
    /// as an egui-managed texture.
    #[cfg(feature = "cpu-readback")]
    fn acquire_frame(&mut self, ctx: &egui::Context) {
        if let Some(rgba) = self.adapter.rendering_context_handle().read_full_frame() {
            let (w, h) = rgba.dimensions();
            let image =
                egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());
            let handle = ctx.load_texture("servo-frame", image, egui::TextureOptions::LINEAR);
            self.status.set_frame(RenderPath::CpuReadback, w, h);
            self.cpu_texture = Some(handle);
        }
    }

    #[cfg(not(feature = "cpu-readback"))]
    fn current_texture_id(&self) -> Option<egui::TextureId> {
        self.texture_id
    }

    #[cfg(feature = "cpu-readback")]
    fn current_texture_id(&self) -> Option<egui::TextureId> {
        self.cpu_texture.as_ref().map(|h| h.id())
    }
}

impl App for ServoEguiApp {
    // egui 0.34: `ui` is the required entry point (the old `update(ctx)` is
    // deprecated). We receive a Ui filling the window; a top Panel carves the
    // nav bar and the remaining Ui is the Servo viewport.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut Frame) {
        let ctx = ui.ctx().clone();

        // Forward input using the previous frame's viewport rect.
        self.forward_input(&ctx);

        // Pump Servo every frame so it keeps making progress (NoopWaker), but
        // only paint+import when Servo actually has a fresh frame, so we never
        // sample a blank/mid-swap buffer.
        self.servo.spin_event_loop();
        if self.frame_ready.swap(false, Ordering::AcqRel) {
            self.webview.paint();
            self.acquire_frame(&ctx);
            // The winit reference waits for the GPU after importing each frame.
            // Without it, eframe may pipeline the next frame's import blit into
            // grafting's reused texture while egui is still sampling it.
            #[cfg(not(feature = "cpu-readback"))]
            {
                let _ = self
                    .render_state
                    .device
                    .poll(wgpu::PollType::wait_indefinitely());
            }
        }

        egui::Panel::top("nav").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("URL:");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.url_input)
                        .desired_width(f32::INFINITY)
                        .hint_text("Enter URL..."),
                );
                self.url_focused = resp.has_focus();
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.navigate();
                }
            });
            ui.label(self.status.summary());
        });

        // Remaining space is the Servo viewport.
        let avail = ui.available_size();
        if let Some(id) = self.current_texture_id() {
            // Stretch the imported texture over the remaining area. grafting
            // returns a top-left-origin texture (it normalizes the import), so
            // the UV is the standard 0..1 with no flip.
            let (rect, _) = ui.allocate_exact_size(avail, egui::Sense::hover());
            ui.painter().image(
                id,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            self.image_rect = rect;
        } else {
            ui.centered_and_justified(|ui| ui.label("Servo loading…"));
        }

        self.sync_viewport(&ctx);

        // Keep repainting so live web content animates.
        ctx.request_repaint();
    }
}

// ── Servo support ────────────────────────────────────────────────────────────

fn map_button(button: egui::PointerButton) -> Option<ServoMouseButton> {
    Some(match button {
        egui::PointerButton::Primary => ServoMouseButton::Left,
        egui::PointerButton::Secondary => ServoMouseButton::Right,
        egui::PointerButton::Middle => ServoMouseButton::Middle,
        egui::PointerButton::Extra1 => ServoMouseButton::Back,
        egui::PointerButton::Extra2 => ServoMouseButton::Forward,
    })
}

/// A no-op waker — we drive continuous repaints via `ctx.request_repaint()`.
#[derive(Clone)]
struct NoopWaker;

impl EventLoopWaker for NoopWaker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }
    fn wake(&self) {}
}

struct DemoDelegate {
    frame_ready: Arc<AtomicBool>,
    egui_ctx: egui::Context,
}

impl WebViewDelegate for DemoDelegate {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        self.frame_ready.store(true, Ordering::Release);
        // Wake egui so the fresh frame is painted+imported promptly.
        self.egui_ctx.request_repaint();
    }

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

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() -> eframe::Result {
    aws_lc_rs::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let initial_url = demo_support::resolve_initial_url(env!("CARGO_MANIFEST_DIR"))
        .expect("failed to resolve initial URL");

    // Force the DX12 backend on Windows: the zero-copy import LUID-matches
    // surfman/ANGLE to the host wgpu device, which requires a DX12 host (the
    // match reads the DX12 adapter LUID). Without this eframe may pick Vulkan
    // and the adapter's new_for_device() would error.
    let mut wgpu_options = egui_wgpu::WgpuConfiguration::default();
    #[cfg(windows)]
    if let egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends = wgpu::Backends::DX12;
    }

    let options = NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([DEFAULT_WIDTH as f32, DEFAULT_HEIGHT as f32])
            .with_title("demo-servo-egui"),
        ..Default::default()
    };

    eframe::run_native(
        "demo-servo-egui",
        options,
        Box::new(move |cc| Ok(Box::new(ServoEguiApp::new(cc, initial_url)))),
    )
}
