//! Demo embedding Servo in a [Slint] UI, zero-copy.
//!
//! Slint owns its wgpu device (femtovg-on-wgpu renderer). This demo uses Slint's
//! official `unstable-wgpu-28` integration: a rendering notifier hands us Slint's
//! `wgpu::Device`/`Queue`, we run Servo on that device, import each frame as a
//! `wgpu::Texture`, and turn it into a `slint::Image` via
//! `slint::Image::try_from(wgpu::Texture)`. No CPU readback.
//!
//! wgpu-graft was originally forked from slint's `examples/servo`; this demo
//! closes the loop, using grafting for the import and Slint's public texture
//! interop for presentation. Windows + DX12 (ANGLE-D3D11 → DX12 import path).
//!
//! Usage:
//!   cargo run -p demo-servo-slint -- https://example.com
//!   cargo run -p demo-servo-slint                     # built-in fixture page

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use euclid::Scale;
use rustls::crypto::aws_lc_rs;
use servo::{EventLoopWaker, Servo, ServoBuilder, WebView, WebViewBuilder, WebViewDelegate};
use servo_wgpu_interop_adapter::ServoWgpuInteropAdapter;
use slint::ComponentHandle;
use url::Url;
use winit::dpi::PhysicalSize;

slint::slint! {
    export component MainWindow inherits Window {
        in property <image> frame;
        title: "demo-servo-slint";
        preferred-width: 1280px;
        preferred-height: 800px;
        Image {
            source: frame;
            width: 100%;
            height: 100%;
            image-fit: fill;
        }
    }
}

struct ServoState {
    servo: Servo,
    webview: WebView,
    interop: ServoWgpuInteropAdapter,
    size: PhysicalSize<u32>,
}

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    aws_lc_rs::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let initial_url = demo_support::resolve_initial_url(env!("CARGO_MANIFEST_DIR"))?;

    // Ask Slint to render through wgpu. On Windows force DX12 + HighPerformance
    // so the ANGLE-D3D11 → DX12 shared-texture import path works and surfman is
    // LUID-matched to the GPU Slint uses.
    let mut settings = slint::wgpu_28::WGPUSettings::default();
    #[cfg(windows)]
    {
        settings.backends = slint::wgpu_28::wgpu::Backends::DX12;
        settings.power_preference = slint::wgpu_28::wgpu::PowerPreference::HighPerformance;
    }
    slint::BackendSelector::new()
        .require_wgpu_28(slint::wgpu_28::WGPUConfiguration::Automatic(settings))
        .select()?;

    let app = MainWindow::new()?;

    let servo_state: Rc<RefCell<Option<ServoState>>> = Rc::new(RefCell::new(None));

    // Create Servo on Slint's wgpu device once it is available (RenderingSetup).
    {
        let state = servo_state.clone();
        let url = initial_url.clone();
        let win_weak = app.as_weak();
        app.window()
            .set_rendering_notifier(move |rendering_state, graphics_api| {
                if !matches!(rendering_state, slint::RenderingState::RenderingSetup)
                    || state.borrow().is_some()
                {
                    return;
                }
                let slint::GraphicsAPI::WGPU28 { device, queue, .. } = graphics_api else {
                    return;
                };

                let size = win_weak
                    .upgrade()
                    .map(|a| {
                        let s = a.window().size();
                        PhysicalSize::new(s.width.max(1), s.height.max(1))
                    })
                    .unwrap_or(PhysicalSize::new(1280, 800));

                match ServoWgpuInteropAdapter::new(device.clone(), queue.clone(), size) {
                    Ok(interop) => {
                        let servo = ServoBuilder::default()
                            .event_loop_waker(Box::new(NoopWaker))
                            .build();
                        servo.setup_logging();
                        let webview = WebViewBuilder::new(&servo, interop.rendering_context())
                            .url(url.clone())
                            .hidpi_scale_factor(Scale::new(1.0))
                            .delegate(Rc::new(DemoDelegate))
                            .build();
                        *state.borrow_mut() = Some(ServoState {
                            servo,
                            webview,
                            interop,
                            size,
                        });
                        println!("[slint] Servo embedded on Slint's wgpu device");
                    }
                    Err(e) => eprintln!("[slint] adapter init failed: {e:?}"),
                }
            })
            .expect("failed to set rendering notifier");
    }

    // Drive Servo and import each frame into a slint::Image.
    let timer = slint::Timer::default();
    {
        let state = servo_state.clone();
        let app_weak = app.as_weak();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(16),
            move || {
                let Some(app) = app_weak.upgrade() else {
                    return;
                };
                let mut guard = state.borrow_mut();
                let Some(st) = guard.as_mut() else {
                    return;
                };

                st.servo.spin_event_loop();

                // Keep Servo's viewport matched to the window.
                let s = app.window().size();
                let cur = PhysicalSize::new(s.width.max(1), s.height.max(1));
                if cur != st.size {
                    st.webview.resize(cur);
                    st.size = cur;
                }

                st.webview.paint();

                match st.interop.import_current_frame_default() {
                    Ok(imported) => match slint::Image::try_from(imported.texture) {
                        Ok(image) => app.set_frame(image),
                        Err(e) => eprintln!("[slint] Image::try_from failed: {e:?}"),
                    },
                    Err(e) => eprintln!("[slint] import failed: {e:?}"),
                }
            },
        );
    }

    app.run()?;
    Ok(())
}
