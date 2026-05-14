use std::{cell::RefCell, num::NonZeroU32, rc::Rc, sync::Arc};

use euclid::default::Size2D;
use gleam::gl::{self, Gl};
use glow::NativeFramebuffer;
use image::RgbaImage;
use surfman::{
    Adapter, Connection, Context, ContextAttributeFlags, ContextAttributes, Device, Error, GLApi,
    NativeWidget, Surface, SurfaceAccess, SurfaceInfo, SurfaceTexture, SurfaceType,
    chains::SwapChain,
};

pub struct SurfmanFrameContext {
    pub gleam_gl: Rc<dyn Gl>,
    pub glow_gl: Arc<glow::Context>,
    pub device: RefCell<Device>,
    pub context: RefCell<Context>,
}

impl Drop for SurfmanFrameContext {
    fn drop(&mut self) {
        let device = &mut self.device.borrow_mut();
        let context = &mut self.context.borrow_mut();
        let _ = device.destroy_context(context);
    }
}

impl SurfmanFrameContext {
    pub fn new(connection: &Connection, adapter: &Adapter) -> Result<Self, Error> {
        let device = connection.create_device(adapter)?;

        let flags = ContextAttributeFlags::ALPHA
            | ContextAttributeFlags::DEPTH
            | ContextAttributeFlags::STENCIL;
        let gl_api = connection.gl_api();
        let version = match &gl_api {
            GLApi::GLES => surfman::GLVersion { major: 3, minor: 0 },
            GLApi::GL => surfman::GLVersion { major: 4, minor: 5 },
        };
        let context_descriptor =
            device.create_context_descriptor(&ContextAttributes { flags, version })?;
        let context = device.create_context(&context_descriptor, None)?;

        let gleam_gl = match gl_api {
            GLApi::GL => unsafe {
                gl::GlFns::load_with(|func_name| device.get_proc_address(&context, func_name))
            },
            GLApi::GLES => unsafe {
                gl::GlesFns::load_with(|func_name| device.get_proc_address(&context, func_name))
            },
        };

        let glow_gl = unsafe {
            glow::Context::from_loader_function(|function_name| {
                device.get_proc_address(&context, function_name)
            })
        };

        Ok(Self {
            gleam_gl,
            glow_gl: Arc::new(glow_gl),
            device: RefCell::new(device),
            context: RefCell::new(context),
        })
    }

    pub fn create_surface(
        &self,
        surface_type: SurfaceType<NativeWidget>,
    ) -> Result<Surface, Error> {
        let device = &mut self.device.borrow_mut();
        let context = &self.context.borrow();
        device.create_surface(context, SurfaceAccess::GPUOnly, surface_type)
    }

    pub fn bind_surface(&self, surface: Surface) -> Result<(), Error> {
        let device = &self.device.borrow();
        let context = &mut self.context.borrow_mut();
        device
            .bind_surface_to_context(context, surface)
            .map_err(|(err, mut surface)| {
                let _ = device.destroy_surface(context, &mut surface);
                err
            })?;
        Ok(())
    }

    pub fn unbind_surface(&self) -> Result<Option<Surface>, Error> {
        let device = &self.device.borrow();
        let context = &mut self.context.borrow_mut();
        device.unbind_surface_from_context(context)
    }

    pub fn create_attached_swap_chain(&self) -> Result<SwapChain<Device>, Error> {
        let device = &mut self.device.borrow_mut();
        let context = &mut self.context.borrow_mut();
        SwapChain::create_attached(device, context, SurfaceAccess::GPUOnly)
    }

    pub fn make_current(&self) -> Result<(), Error> {
        let device = &self.device.borrow();
        let context = &mut self.context.borrow_mut();
        device.make_context_current(context)
    }

    pub fn prepare_for_rendering(&self) {
        let framebuffer_id = self
            .framebuffer()
            .map_or(0, |framebuffer| framebuffer.0.into());
        self.gleam_gl
            .bind_framebuffer(gleam::gl::FRAMEBUFFER, framebuffer_id);
    }

    pub fn read_to_image_region(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Option<RgbaImage> {
        let framebuffer_id = self
            .framebuffer()
            .map_or(0, |framebuffer| framebuffer.0.into());
        Self::read_framebuffer_to_image(&self.gleam_gl, framebuffer_id, x, y, width, height)
    }

    pub fn create_texture(&self, surface: Surface) -> Option<(SurfaceTexture, u32, Size2D<i32>)> {
        let device = &self.device.borrow();
        let context = &mut self.context.borrow_mut();

        let SurfaceInfo {
            id: _front_buffer_id,
            size,
            ..
        } = device.surface_info(&surface);
        let surface_texture = device.create_surface_texture(context, surface).ok()?;
        let gl_texture = device
            .surface_texture_object(&surface_texture)
            .map(|tex| tex.0.get())
            .unwrap_or(0);

        Some((surface_texture, gl_texture, size))
    }

    pub fn destroy_texture(&self, surface_texture: SurfaceTexture) -> Option<Surface> {
        let device = &self.device.borrow();
        let context = &mut self.context.borrow_mut();

        device
            .destroy_surface_texture(context, surface_texture)
            .map_err(|(error, _)| error)
            .ok()
    }

    pub fn connection(&self) -> Option<Connection> {
        Some(self.device.borrow().connection())
    }

    fn framebuffer(&self) -> Option<NativeFramebuffer> {
        let device = &self.device.borrow();
        let context = &self.context.borrow();
        device
            .context_surface_info(context)
            .unwrap_or(None)
            .and_then(|info| info.framebuffer_object)
            .and_then(|framebuffer| NonZeroU32::new(framebuffer.0.get()))
            .map(NativeFramebuffer)
    }

    fn read_framebuffer_to_image(
        gl: &Rc<dyn Gl>,
        framebuffer_id: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Option<RgbaImage> {
        gl.bind_framebuffer(gl::FRAMEBUFFER, framebuffer_id);
        gl.bind_vertex_array(0);

        let mut pixels = gl.read_pixels(x, y, width, height, gl::RGBA, gl::UNSIGNED_BYTE);

        if gl.get_error() != gl::NO_ERROR {
            return None;
        }

        let width = width as usize;
        let height = height as usize;
        let orig_pixels = pixels.clone();
        let stride = width * 4;
        for row in 0..height {
            let dst_start = row * stride;
            let src_start = (height - row - 1) * stride;
            let src_slice = &orig_pixels[src_start..src_start + stride];
            pixels[dst_start..dst_start + stride].clone_from_slice(src_slice);
        }

        RgbaImage::from_raw(width as u32, height as u32, pixels)
    }
}
