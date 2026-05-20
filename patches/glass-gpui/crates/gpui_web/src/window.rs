use crate::display::WebDisplay;
use crate::events::{ClickState, WebEventListeners, is_mac_platform};
use crate::window_environment::{CanvasMetrics, ResizeUpdate, WindowEnvironmentState};
use std::sync::Arc;
use std::{cell::Cell, cell::RefCell, rc::Rc};

use gpui::{
    AnyWindowHandle, Bounds, Capslock, Decorations, DevicePixels, DispatchEventResult, GpuSpecs,
    Modifiers, MouseButton, Pixels, PlatformAtlas, PlatformDisplay, PlatformInput,
    PlatformInputHandler, PlatformWindow, Point, PromptButton, PromptLevel, RequestFrameOptions,
    ResizeEdge, Scene, Size, WindowAppearance, WindowBackgroundAppearance, WindowBounds,
    WindowControlArea, WindowControls, WindowDecorations, WindowParams, px,
};
use gpui_wgpu::{WgpuContext, WgpuRenderer, WgpuSurfaceConfig};
use wasm_bindgen::prelude::*;

#[derive(Default)]
pub(crate) struct WebWindowCallbacks {
    pub(crate) request_frame: Option<Box<dyn FnMut(RequestFrameOptions)>>,
    pub(crate) input: Option<Box<dyn FnMut(PlatformInput) -> DispatchEventResult>>,
    pub(crate) active_status_change: Option<Box<dyn FnMut(bool)>>,
    pub(crate) hover_status_change: Option<Box<dyn FnMut(bool)>>,
    pub(crate) resize: Option<Box<dyn FnMut(Size<Pixels>, f32)>>,
    pub(crate) moved: Option<Box<dyn FnMut()>>,
    pub(crate) should_close: Option<Box<dyn FnMut() -> bool>>,
    pub(crate) close: Option<Box<dyn FnOnce()>>,
    pub(crate) appearance_changed: Option<Box<dyn FnMut()>>,
    pub(crate) hit_test_window_control: Option<Box<dyn FnMut() -> Option<WindowControlArea>>>,
}

pub(crate) struct WebWindowMutableState {
    pub(crate) renderer: WgpuRenderer,
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) scale_factor: f32,
    pub(crate) max_texture_dimension: u32,
    pub(crate) title: String,
    pub(crate) input_handler: Option<PlatformInputHandler>,
    pub(crate) is_fullscreen: bool,
    pub(crate) is_active: bool,
    pub(crate) is_hovered: bool,
    pub(crate) mouse_position: Point<Pixels>,
    pub(crate) modifiers: Modifiers,
    pub(crate) capslock: Capslock,
}

pub(crate) struct WebWindowInner {
    pub(crate) browser_window: web_sys::Window,
    pub(crate) canvas: web_sys::HtmlCanvasElement,
    pub(crate) input_element: web_sys::HtmlInputElement,
    pub(crate) has_device_pixel_support: bool,
    pub(crate) is_mac: bool,
    pub(crate) state: RefCell<WebWindowMutableState>,
    pub(crate) callbacks: RefCell<WebWindowCallbacks>,
    pub(crate) click_state: RefCell<ClickState>,
    pub(crate) pressed_button: Cell<Option<MouseButton>>,
    environment: RefCell<WindowEnvironmentState>,
    pub(crate) is_composing: Cell<bool>,
    dpr_watch: RefCell<Option<MediaQuerySubscription>>,
    pending_physical_size: Cell<Option<(u32, u32)>>,
}

pub struct WebWindow {
    inner: Rc<WebWindowInner>,
    display: Rc<dyn PlatformDisplay>,
    #[allow(dead_code)]
    handle: AnyWindowHandle,
    _raf_closure: Closure<dyn FnMut()>,
    _resize_observer: Option<web_sys::ResizeObserver>,
    _resize_observer_closure: Closure<dyn FnMut(js_sys::Array)>,
    _event_listeners: WebEventListeners,
}

impl WebWindow {
    pub fn new(
        handle: AnyWindowHandle,
        _params: WindowParams,
        context: &WgpuContext,
        browser_window: web_sys::Window,
    ) -> anyhow::Result<Self> {
        let document = browser_window
            .document()
            .ok_or_else(|| anyhow::anyhow!("No `document` found on window"))?;

        let canvas: web_sys::HtmlCanvasElement = document
            .create_element("canvas")
            .map_err(|e| anyhow::anyhow!("Failed to create canvas element: {e:?}"))?
            .dyn_into()
            .map_err(|e| anyhow::anyhow!("Created element is not a canvas: {e:?}"))?;

        let dpr = browser_window.device_pixel_ratio() as f32;
        let max_texture_dimension = context.device.limits().max_texture_dimension_2d;
        let has_device_pixel_support = check_device_pixel_support();

        canvas.set_tab_index(-1);

        let style = canvas.style();
        style
            .set_property("width", "100%")
            .map_err(|e| anyhow::anyhow!("Failed to set canvas width style: {e:?}"))?;
        style
            .set_property("height", "100%")
            .map_err(|e| anyhow::anyhow!("Failed to set canvas height style: {e:?}"))?;
        style
            .set_property("display", "block")
            .map_err(|e| anyhow::anyhow!("Failed to set canvas display style: {e:?}"))?;
        style
            .set_property("outline", "none")
            .map_err(|e| anyhow::anyhow!("Failed to set canvas outline style: {e:?}"))?;
        style
            .set_property("touch-action", "none")
            .map_err(|e| anyhow::anyhow!("Failed to set touch-action style: {e:?}"))?;

        let body = document
            .body()
            .ok_or_else(|| anyhow::anyhow!("No `body` found on document"))?;
        body.append_child(&canvas)
            .map_err(|e| anyhow::anyhow!("Failed to append canvas to body: {e:?}"))?;

        let input_element: web_sys::HtmlInputElement = document
            .create_element("input")
            .map_err(|e| anyhow::anyhow!("Failed to create input element: {e:?}"))?
            .dyn_into()
            .map_err(|e| anyhow::anyhow!("Created element is not an input: {e:?}"))?;
        let input_style = input_element.style();
        input_style.set_property("position", "fixed").ok();
        input_style.set_property("top", "0").ok();
        input_style.set_property("left", "0").ok();
        input_style.set_property("width", "1px").ok();
        input_style.set_property("height", "1px").ok();
        input_style.set_property("opacity", "0").ok();
        body.append_child(&input_element)
            .map_err(|e| anyhow::anyhow!("Failed to append input to body: {e:?}"))?;
        input_element.focus().ok();

        let device_size = Size {
            width: DevicePixels(0),
            height: DevicePixels(0),
        };

        let renderer_config = WgpuSurfaceConfig {
            size: device_size,
            transparent: false,
            preferred_present_mode: None,
        };

        let renderer = WgpuRenderer::new_from_canvas(context, &canvas, renderer_config)?;

        let display: Rc<dyn PlatformDisplay> = Rc::new(WebDisplay::new(browser_window.clone()));

        let initial_bounds = Bounds {
            origin: Point::default(),
            size: Size::default(),
        };

        let mutable_state = WebWindowMutableState {
            renderer,
            bounds: initial_bounds,
            scale_factor: dpr,
            max_texture_dimension,
            title: String::new(),
            input_handler: None,
            is_fullscreen: false,
            is_active: true,
            is_hovered: false,
            mouse_position: Point::default(),
            modifiers: Modifiers::default(),
            capslock: Capslock::default(),
        };

        let is_mac = is_mac_platform(&browser_window);

        let inner = Rc::new(WebWindowInner {
            browser_window,
            canvas,
            input_element,
            has_device_pixel_support,
            is_mac,
            state: RefCell::new(mutable_state),
            callbacks: RefCell::new(WebWindowCallbacks::default()),
            click_state: RefCell::new(ClickState::default()),
            pressed_button: Cell::new(None),
            environment: RefCell::new(WindowEnvironmentState::new(dpr as f64)),
            is_composing: Cell::new(false),
            dpr_watch: RefCell::new(None),
            pending_physical_size: Cell::new(None),
        });

        let raf_closure = inner.create_raf_closure();
        inner.schedule_raf(&raf_closure);

        let resize_observer_closure = Self::create_resize_observer_closure(Rc::clone(&inner));
        let resize_observer =
            web_sys::ResizeObserver::new(resize_observer_closure.as_ref().unchecked_ref()).ok();

        if let Some(ref observer) = resize_observer {
            inner.observe_canvas(observer);
            inner.bind_dpr_watch();
        }

        let event_listeners = inner.register_event_listeners();

        Ok(Self {
            inner,
            display,
            handle,
            _raf_closure: raf_closure,
            _resize_observer: resize_observer,
            _resize_observer_closure: resize_observer_closure,
            _event_listeners: event_listeners,
        })
    }

    fn create_resize_observer_closure(
        inner: Rc<WebWindowInner>,
    ) -> Closure<dyn FnMut(js_sys::Array)> {
        Closure::new(move |entries: js_sys::Array| {
            let entry: web_sys::ResizeObserverEntry = match entries.get(0).dyn_into().ok() {
                Some(entry) => entry,
                None => return,
            };
            inner
                .environment
                .borrow_mut()
                .queue_resize(inner.metrics_from_resize_entry(&entry));
        })
    }
}

impl WebWindowInner {
    fn create_raf_closure(self: &Rc<Self>) -> Closure<dyn FnMut()> {
        let raf_handle: Rc<RefCell<Option<js_sys::Function>>> = Rc::new(RefCell::new(None));
        let raf_handle_inner = Rc::clone(&raf_handle);

        let this = Rc::clone(self);
        let closure = Closure::new(move || {
            this.reconcile_environment();

            {
                let mut callbacks = this.callbacks.borrow_mut();
                if let Some(ref mut callback) = callbacks.request_frame {
                    callback(RequestFrameOptions {
                        require_presentation: true,
                        force_render: false,
                    });
                }
            }

            // Re-schedule for the next frame
            if let Some(ref func) = *raf_handle_inner.borrow() {
                this.browser_window.request_animation_frame(func).ok();
            }
        });

        let js_func: js_sys::Function =
            closure.as_ref().unchecked_ref::<js_sys::Function>().clone();
        *raf_handle.borrow_mut() = Some(js_func);

        closure
    }

    fn schedule_raf(&self, closure: &Closure<dyn FnMut()>) {
        self.browser_window
            .request_animation_frame(closure.as_ref().unchecked_ref())
            .ok();
    }

    fn observe_canvas(&self, observer: &web_sys::ResizeObserver) {
        observer.unobserve(&self.canvas);
        if self.has_device_pixel_support {
            let options = web_sys::ResizeObserverOptions::new();
            options.set_box(web_sys::ResizeObserverBoxOptions::DevicePixelContentBox);
            observer.observe_with_options(&self.canvas, &options);
        } else {
            observer.observe(&self.canvas);
        }
    }

    fn metrics_from_resize_entry(&self, entry: &web_sys::ResizeObserverEntry) -> CanvasMetrics {
        let dpr = self.browser_window.device_pixel_ratio();
        let scale_factor = dpr as f32;

        let (physical_width, physical_height, logical_width, logical_height) =
            if self.has_device_pixel_support {
                let size: web_sys::ResizeObserverSize = entry
                    .device_pixel_content_box_size()
                    .get(0)
                    .unchecked_into();
                let physical_width = size.inline_size() as u32;
                let physical_height = size.block_size() as u32;
                let logical_width = physical_width as f64 / dpr;
                let logical_height = physical_height as f64 / dpr;
                (
                    physical_width,
                    physical_height,
                    logical_width as f32,
                    logical_height as f32,
                )
            } else {
                // Safari fallback: use contentRect (always CSS px).
                let rect = entry.content_rect();
                let logical_width = rect.width() as f32;
                let logical_height = rect.height() as f32;
                let physical_width = (logical_width as f64 * dpr).round() as u32;
                let physical_height = (logical_height as f64 * dpr).round() as u32;
                (
                    physical_width,
                    physical_height,
                    logical_width,
                    logical_height,
                )
            };

        CanvasMetrics {
            physical_width,
            physical_height,
            logical_width,
            logical_height,
            scale_factor,
        }
    }

    fn measure_canvas_metrics(&self) -> CanvasMetrics {
        let rect = self.canvas.get_bounding_client_rect();
        let dpr = self.browser_window.device_pixel_ratio();
        let logical_width = rect.width() as f32;
        let logical_height = rect.height() as f32;
        let physical_width = (logical_width as f64 * dpr).round() as u32;
        let physical_height = (logical_height as f64 * dpr).round() as u32;

        CanvasMetrics {
            physical_width,
            physical_height,
            logical_width,
            logical_height,
            scale_factor: dpr as f32,
        }
    }

    fn reconcile_environment(self: &Rc<Self>) {
        let current_dpr = self.browser_window.device_pixel_ratio();
        let measured_metrics = {
            let environment = self.environment.borrow();
            environment
                .needs_measurement()
                .then(|| self.measure_canvas_metrics())
        };
        let max_texture_dimension = self.state.borrow().max_texture_dimension;
        let update = self.environment.borrow_mut().reconcile(
            current_dpr,
            measured_metrics,
            max_texture_dimension,
        );

        if update.media_query.is_some() {
            self.bind_dpr_watch();
        }

        if let Some(resize) = update.resize {
            self.apply_resize_update(resize);
        }
    }

    fn apply_resize_update(&self, resize: ResizeUpdate) {
        match resize {
            ResizeUpdate::Hidden { scale_factor } => {
                self.pending_physical_size.set(None);

                let mut state = self.state.borrow_mut();
                state.bounds.size = Size::default();
                state.scale_factor = scale_factor;
                drop(state);

                let mut callbacks = self.callbacks.borrow_mut();
                if let Some(ref mut callback) = callbacks.resize {
                    callback(Size::default(), scale_factor);
                }
            }
            ResizeUpdate::Visible {
                logical_width,
                logical_height,
                physical_width,
                physical_height,
                scale_factor,
            } => {
                self.pending_physical_size
                    .set(Some((physical_width, physical_height)));

                let new_size = Size {
                    width: px(logical_width),
                    height: px(logical_height),
                };

                {
                    let mut state = self.state.borrow_mut();
                    state.bounds.size = new_size;
                    state.scale_factor = scale_factor;
                }

                let mut callbacks = self.callbacks.borrow_mut();
                if let Some(ref mut callback) = callbacks.resize {
                    callback(new_size, scale_factor);
                }
            }
        }
    }

    fn bind_dpr_watch(self: &Rc<Self>) {
        let query = {
            let environment = self.environment.borrow();
            environment.current_media_query().map(str::to_owned)
        };
        let Some(query) = query else {
            return;
        };

        let this = Rc::clone(self);
        *self.dpr_watch.borrow_mut() =
            MediaQuerySubscription::new(&self.browser_window, &query, move |_event| {
                this.environment.borrow_mut().queue_dpr_change();
            });
    }

    pub(crate) fn register_visibility_change(
        self: &Rc<Self>,
    ) -> Option<Closure<dyn FnMut(JsValue)>> {
        let document = self.browser_window.document()?;
        let this = Rc::clone(self);

        let closure = Closure::<dyn FnMut(JsValue)>::new(move |_event: JsValue| {
            let is_visible = this
                .browser_window
                .document()
                .map(|doc| {
                    let state_str: String = js_sys::Reflect::get(&doc, &"visibilityState".into())
                        .ok()
                        .and_then(|v| v.as_string())
                        .unwrap_or_default();
                    state_str == "visible"
                })
                .unwrap_or(true);

            {
                let mut state = this.state.borrow_mut();
                state.is_active = is_visible;
            }
            let mut callbacks = this.callbacks.borrow_mut();
            if let Some(ref mut callback) = callbacks.active_status_change {
                callback(is_visible);
            }
        });

        document
            .add_event_listener_with_callback("visibilitychange", closure.as_ref().unchecked_ref())
            .ok();

        Some(closure)
    }

    pub(crate) fn with_input_handler<R>(
        &self,
        f: impl FnOnce(&mut PlatformInputHandler) -> R,
    ) -> Option<R> {
        let mut handler = self.state.borrow_mut().input_handler.take()?;
        let result = f(&mut handler);
        self.state.borrow_mut().input_handler = Some(handler);
        Some(result)
    }

    pub(crate) fn register_appearance_change(
        self: &Rc<Self>,
    ) -> Option<Closure<dyn FnMut(JsValue)>> {
        let mql = self
            .browser_window
            .match_media("(prefers-color-scheme: dark)")
            .ok()??;

        let this = Rc::clone(self);
        let closure = Closure::<dyn FnMut(JsValue)>::new(move |_event: JsValue| {
            let mut callbacks = this.callbacks.borrow_mut();
            if let Some(ref mut callback) = callbacks.appearance_changed {
                callback();
            }
        });

        mql.add_event_listener_with_callback("change", closure.as_ref().unchecked_ref())
            .ok();

        Some(closure)
    }
}

fn current_appearance(browser_window: &web_sys::Window) -> WindowAppearance {
    let is_dark = browser_window
        .match_media("(prefers-color-scheme: dark)")
        .ok()
        .flatten()
        .map(|mql| mql.matches())
        .unwrap_or(false);

    if is_dark {
        WindowAppearance::Dark
    } else {
        WindowAppearance::Light
    }
}

struct MediaQuerySubscription {
    mql: web_sys::MediaQueryList,
    callback: Closure<dyn FnMut(JsValue)>,
}

impl MediaQuerySubscription {
    fn new(
        browser_window: &web_sys::Window,
        query: &str,
        handler: impl FnMut(JsValue) + 'static,
    ) -> Option<Self> {
        let mql = browser_window.match_media(query).ok().flatten()?;
        let callback = Closure::<dyn FnMut(JsValue)>::new(handler);
        mql.add_event_listener_with_callback("change", callback.as_ref().unchecked_ref())
            .ok()?;

        Some(Self { mql, callback })
    }
}

impl Drop for MediaQuerySubscription {
    fn drop(&mut self) {
        self.mql
            .remove_event_listener_with_callback("change", self.callback.as_ref().unchecked_ref())
            .ok();
    }
}

// Safari does not support `devicePixelContentBoxSize`, so detect whether it's available.
fn check_device_pixel_support() -> bool {
    let global: JsValue = js_sys::global().into();
    let Ok(constructor) = js_sys::Reflect::get(&global, &"ResizeObserverEntry".into()) else {
        return false;
    };
    let Ok(prototype) = js_sys::Reflect::get(&constructor, &"prototype".into()) else {
        return false;
    };
    let descriptor = js_sys::Object::get_own_property_descriptor(
        &prototype.unchecked_into::<js_sys::Object>(),
        &"devicePixelContentBoxSize".into(),
    );
    !descriptor.is_undefined()
}

impl raw_window_handle::HasWindowHandle for WebWindow {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let canvas_ref: &JsValue = self.inner.canvas.as_ref();
        let obj = std::ptr::NonNull::from(canvas_ref).cast::<std::ffi::c_void>();
        let handle = raw_window_handle::WebCanvasWindowHandle::new(obj);
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(handle.into()) })
    }
}

impl raw_window_handle::HasDisplayHandle for WebWindow {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(raw_window_handle::DisplayHandle::web())
    }
}

impl PlatformWindow for WebWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        self.inner.state.borrow().bounds
    }

    fn is_maximized(&self) -> bool {
        false
    }

    fn window_bounds(&self) -> WindowBounds {
        WindowBounds::Windowed(self.bounds())
    }

    fn content_size(&self) -> Size<Pixels> {
        self.inner.state.borrow().bounds.size
    }

    fn resize(&mut self, size: Size<Pixels>) {
        let style = self.inner.canvas.style();
        style
            .set_property("width", &format!("{}px", f32::from(size.width)))
            .ok();
        style
            .set_property("height", &format!("{}px", f32::from(size.height)))
            .ok();
    }

    fn scale_factor(&self) -> f32 {
        self.inner.state.borrow().scale_factor
    }

    fn appearance(&self) -> WindowAppearance {
        current_appearance(&self.inner.browser_window)
    }

    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(self.display.clone())
    }

    fn mouse_position(&self) -> Point<Pixels> {
        self.inner.state.borrow().mouse_position
    }

    fn modifiers(&self) -> Modifiers {
        self.inner.state.borrow().modifiers
    }

    fn capslock(&self) -> Capslock {
        self.inner.state.borrow().capslock
    }

    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        self.inner.state.borrow_mut().input_handler = Some(input_handler);
    }

    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.inner.state.borrow_mut().input_handler.take()
    }

    fn prompt(
        &self,
        _level: PromptLevel,
        _msg: &str,
        _detail: Option<&str>,
        _answers: &[PromptButton],
    ) -> Option<futures::channel::oneshot::Receiver<usize>> {
        None
    }

    fn activate(&self) {
        self.inner.state.borrow_mut().is_active = true;
    }

    fn is_active(&self) -> bool {
        self.inner.state.borrow().is_active
    }

    fn is_hovered(&self) -> bool {
        self.inner.state.borrow().is_hovered
    }

    fn background_appearance(&self) -> WindowBackgroundAppearance {
        WindowBackgroundAppearance::Opaque
    }

    fn set_title(&mut self, title: &str) {
        self.inner.state.borrow_mut().title = title.to_owned();
        if let Some(document) = self.inner.browser_window.document() {
            document.set_title(title);
        }
    }

    fn set_background_appearance(&self, _background: WindowBackgroundAppearance) {}

    fn minimize(&self) {
        log::warn!("WebWindow::minimize is not supported in the browser");
    }

    fn zoom(&self) {
        log::warn!("WebWindow::zoom is not supported in the browser");
    }

    fn toggle_fullscreen(&self) {
        let mut state = self.inner.state.borrow_mut();
        state.is_fullscreen = !state.is_fullscreen;

        if state.is_fullscreen {
            let canvas: &web_sys::Element = self.inner.canvas.as_ref();
            canvas.request_fullscreen().ok();
        } else {
            if let Some(document) = self.inner.browser_window.document() {
                document.exit_fullscreen();
            }
        }
    }

    fn is_fullscreen(&self) -> bool {
        self.inner.state.borrow().is_fullscreen
    }

    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        self.inner.callbacks.borrow_mut().request_frame = Some(callback);
    }

    fn on_input(&self, callback: Box<dyn FnMut(PlatformInput) -> DispatchEventResult>) {
        self.inner.callbacks.borrow_mut().input = Some(callback);
    }

    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.inner.callbacks.borrow_mut().active_status_change = Some(callback);
    }

    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.inner.callbacks.borrow_mut().hover_status_change = Some(callback);
    }

    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        self.inner.callbacks.borrow_mut().resize = Some(callback);
    }

    fn on_moved(&self, callback: Box<dyn FnMut()>) {
        self.inner.callbacks.borrow_mut().moved = Some(callback);
    }

    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        self.inner.callbacks.borrow_mut().should_close = Some(callback);
    }

    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        self.inner.callbacks.borrow_mut().close = Some(callback);
    }

    fn on_hit_test_window_control(&self, callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
        self.inner.callbacks.borrow_mut().hit_test_window_control = Some(callback);
    }

    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        self.inner.callbacks.borrow_mut().appearance_changed = Some(callback);
    }

    fn draw(&self, scene: &Scene) {
        if let Some((width, height)) = self.inner.pending_physical_size.take() {
            if self.inner.canvas.width() != width || self.inner.canvas.height() != height {
                self.inner.canvas.set_width(width);
                self.inner.canvas.set_height(height);
            }

            let mut state = self.inner.state.borrow_mut();
            state.renderer.update_drawable_size(Size {
                width: DevicePixels(width as i32),
                height: DevicePixels(height as i32),
            });
            drop(state);
        }

        self.inner.state.borrow_mut().renderer.draw(scene);
    }

    fn completed_frame(&self) {
        // On web, presentation happens automatically via wgpu surface present
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.inner.state.borrow().renderer.sprite_atlas().clone()
    }

    fn is_subpixel_rendering_supported(&self) -> bool {
        self.inner
            .state
            .borrow()
            .renderer
            .supports_dual_source_blending()
    }

    fn gpu_specs(&self) -> Option<GpuSpecs> {
        Some(self.inner.state.borrow().renderer.gpu_specs())
    }

    fn update_ime_position(&self, _bounds: Bounds<Pixels>) {}

    fn request_decorations(&self, _decorations: WindowDecorations) {}

    fn show_window_menu(&self, _position: Point<Pixels>) {}

    fn start_window_move(&self) {}

    fn start_window_resize(&self, _edge: ResizeEdge) {}

    fn window_decorations(&self) -> Decorations {
        Decorations::Server
    }

    fn set_app_id(&mut self, _app_id: &str) {}

    fn window_controls(&self) -> WindowControls {
        WindowControls {
            fullscreen: true,
            maximize: false,
            minimize: false,
            window_menu: false,
        }
    }

    fn set_client_inset(&self, _inset: Pixels) {}
}
