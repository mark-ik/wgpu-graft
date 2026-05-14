//! Minimal demo: raw GL rendering imported into a wgpu host.
//!
//! This demonstrates using `grafting` without surfman.
//! A GL context renders a spinning triangle to an offscreen FBO, which is
//! imported into a wgpu texture and presented in a winit window.
//!
//! Requires a Vulkan-capable GPU (the import path uses Vulkan external memory).

use std::borrow::Cow;
use std::ffi::CString;
use std::num::NonZeroU32;
use std::sync::Arc;

use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext, Version};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{SurfaceAttributesBuilder, WindowSurface};
use raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::Window;

use grafting::{
    FrameProducer, HostWgpuContext, ImportOptions, TextureImporter, WgpuTextureImporter,
    raw_gl::producer::RawGlFrameProducer,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    let mut app = App { state: None };
    Ok(event_loop.run_app(&mut app)?)
}

struct App {
    state: Option<AppState>,
}

struct AppState {
    window: Arc<Window>,
    // GL side
    gl: Arc<glow::Context>,
    _gl_context: PossiblyCurrentContext,
    _gl_surface: glutin::surface::Surface<WindowSurface>,
    gl_fbo: glow::NativeFramebuffer,
    gl_color_tex: glow::NativeTexture,
    gl_program: glow::NativeProgram,
    gl_vao: glow::NativeVertexArray,
    // wgpu side
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    importer: WgpuTextureImporter,
    producer: RawGlFrameProducer,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group_layout: wgpu::BindGroupLayout,
    blit_sampler: wgpu::Sampler,
    // state
    frame_count: u64,
    fb_size: PhysicalSize<u32>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match AppState::new(event_loop) {
            Ok(state) => self.state = Some(state),
            Err(err) => {
                eprintln!("failed to initialize: {err}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = &mut self.state else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                if let Err(err) = state.render_frame() {
                    eprintln!("render error: {err}");
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(new_size) => {
                state.resize(new_size);
                state.window.request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }
}

impl AppState {
    fn new(event_loop: &ActiveEventLoop) -> Result<Self, Box<dyn std::error::Error>> {
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("demo-raw-gl (grafting)")
                    .with_inner_size(PhysicalSize::new(800, 600)),
            )?,
        );

        let fb_size = window.inner_size();
        let rwh = window.window_handle()?.as_raw();

        // --- Create GL context via glutin ---
        let display_builder = glutin_winit::DisplayBuilder::new();
        let template = ConfigTemplateBuilder::new().with_alpha_size(8);
        let (_, gl_config) = display_builder
            .build(event_loop, template, |configs| {
                configs
                    .reduce(|a, b| {
                        if a.num_samples() > b.num_samples() {
                            a
                        } else {
                            b
                        }
                    })
                    .unwrap()
            })
            .map_err(|e| format!("glutin display: {e}"))?;

        let gl_display = gl_config.display();
        let context_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(Some(Version::new(4, 5))))
            .build(Some(rwh));

        let not_current_context = unsafe {
            gl_display
                .create_context(&gl_config, &context_attrs)
                .or_else(|_| {
                    let fallback = ContextAttributesBuilder::new()
                        .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
                        .build(Some(rwh));
                    gl_display.create_context(&gl_config, &fallback)
                })?
        };

        let surface_attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            rwh,
            NonZeroU32::new(fb_size.width.max(1)).unwrap(),
            NonZeroU32::new(fb_size.height.max(1)).unwrap(),
        );
        let gl_surface = unsafe { gl_display.create_window_surface(&gl_config, &surface_attrs)? };
        let gl_context = not_current_context.make_current(&gl_surface)?;

        let gl = Arc::new(unsafe {
            glow::Context::from_loader_function(|name| {
                let name = CString::new(name).unwrap();
                gl_display.get_proc_address(&name) as *const _
            })
        });

        println!(
            "GL: {} / {}",
            unsafe { gl.get_parameter_string(glow::RENDERER) },
            unsafe { gl.get_parameter_string(glow::VERSION) },
        );

        // --- Create offscreen FBO for GL rendering ---
        let (gl_color_tex, gl_fbo) = unsafe {
            let tex = gl.create_texture().unwrap();
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                fb_size.width as i32,
                fb_size.height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );

            let fbo = gl.create_framebuffer().unwrap();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );
            (tex, fbo)
        };

        // --- Simple GL triangle program ---
        let (gl_program, gl_vao) = unsafe {
            let vs = gl.create_shader(glow::VERTEX_SHADER).unwrap();
            gl.shader_source(
                vs,
                r#"#version 330 core
                uniform float u_angle;
                const vec2 verts[3] = vec2[3](
                    vec2(0.0, 0.5),
                    vec2(-0.433, -0.25),
                    vec2(0.433, -0.25)
                );
                const vec3 colors[3] = vec3[3](
                    vec3(1.0, 0.2, 0.2),
                    vec3(0.2, 1.0, 0.2),
                    vec3(0.2, 0.2, 1.0)
                );
                out vec3 v_color;
                void main() {
                    float c = cos(u_angle);
                    float s = sin(u_angle);
                    vec2 p = verts[gl_VertexID];
                    gl_Position = vec4(p.x * c - p.y * s, p.x * s + p.y * c, 0.0, 1.0);
                    v_color = colors[gl_VertexID];
                }
                "#,
            );
            gl.compile_shader(vs);

            let fs = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
            gl.shader_source(
                fs,
                r#"#version 330 core
                in vec3 v_color;
                out vec4 frag_color;
                void main() {
                    frag_color = vec4(v_color, 1.0);
                }
                "#,
            );
            gl.compile_shader(fs);

            let program = gl.create_program().unwrap();
            gl.attach_shader(program, vs);
            gl.attach_shader(program, fs);
            gl.link_program(program);
            gl.delete_shader(vs);
            gl.delete_shader(fs);

            let vao = gl.create_vertex_array().unwrap();
            (program, vao)
        };

        // --- Create wgpu device with Vulkan backend ---
        let wgpu_instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });

        let wgpu_surface = wgpu_instance.create_surface(window.clone())?;

        let (adapter, device, queue) = pollster::block_on(async {
            let adapter = wgpu_instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&wgpu_surface),
                    force_fallback_adapter: false,
                })
                .await
                .map_err(|e| format!("adapter: {e}"))?;

            println!("wgpu adapter: {}", adapter.get_info().name);

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("demo-raw-gl"),
                    ..Default::default()
                })
                .await
                .map_err(|e| format!("device: {e}"))?;

            Ok::<_, String>((adapter, device, queue))
        })?;

        let caps = wgpu_surface.get_capabilities(&adapter);
        let surface_format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: fb_size.width.max(1),
            height: fb_size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        wgpu_surface.configure(&device, &surface_config);

        // --- Blit pipeline (fullscreen triangle sampling imported texture) ---
        let blit_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("blit-layout"),
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

        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blit-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit-shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(
                r#"
                struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
                @vertex fn vs(@builtin(vertex_index) i: u32) -> VOut {
                    var p = array<vec2<f32>,3>(vec2(-1.0,-3.0), vec2(-1.0,1.0), vec2(3.0,1.0));
                    var u = array<vec2<f32>,3>(vec2(0.0,2.0), vec2(0.0,0.0), vec2(2.0,0.0));
                    var o: VOut; o.pos = vec4(p[i], 0.0, 1.0); o.uv = u[i]; return o;
                }
                @group(0) @binding(0) var tex: texture_2d<f32>;
                @group(0) @binding(1) var samp: sampler;
                @fragment fn fs(v: VOut) -> @location(0) vec4<f32> {
                    return textureSample(tex, samp, v.uv);
                }
            "#,
            )),
        });

        let blit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&blit_bind_group_layout)],
            immediate_size: 0,
        });

        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit-pipeline"),
            layout: Some(&blit_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs"),
                targets: &[Some(surface_format.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        // --- Interop setup ---
        let host = HostWgpuContext::new(device.clone(), queue.clone());
        let capabilities = host.capabilities();
        println!("interop capabilities: {capabilities:?}");
        let importer = WgpuTextureImporter::new(host);

        let fbo_id = gl_fbo.0.get();
        let producer = RawGlFrameProducer::new(
            gl.clone(),
            {
                let display = gl_display.clone();
                move |name| {
                    let name = CString::new(name).unwrap();
                    display.get_proc_address(&name) as *const _
                }
            },
            fbo_id,
            fb_size,
        );

        Ok(Self {
            window,
            gl,
            _gl_context: gl_context,
            _gl_surface: gl_surface,
            gl_fbo,
            gl_color_tex,
            gl_program,
            gl_vao,
            surface: wgpu_surface,
            device,
            queue,
            surface_config,
            importer,
            producer,
            blit_pipeline,
            blit_bind_group_layout,
            blit_sampler,
            frame_count: 0,
            fb_size,
        })
    }

    fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.fb_size = new_size;
        self.surface_config.width = new_size.width;
        self.surface_config.height = new_size.height;
        self.surface.configure(&self.device, &self.surface_config);
        self.producer.set_size(new_size);

        unsafe {
            self.gl
                .bind_texture(glow::TEXTURE_2D, Some(self.gl_color_tex));
            self.gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                new_size.width as i32,
                new_size.height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
        }
    }

    fn render_frame(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.frame_count += 1;
        let angle = self.frame_count as f32 * 0.02;

        // GL: render spinning triangle to offscreen FBO
        unsafe {
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(self.gl_fbo));
            self.gl
                .viewport(0, 0, self.fb_size.width as i32, self.fb_size.height as i32);
            self.gl.clear_color(0.1, 0.1, 0.15, 1.0);
            self.gl.clear(glow::COLOR_BUFFER_BIT);

            self.gl.use_program(Some(self.gl_program));
            let loc = self.gl.get_uniform_location(self.gl_program, "u_angle");
            self.gl.uniform_1_f32(loc.as_ref(), angle);

            self.gl.bind_vertex_array(Some(self.gl_vao));
            self.gl.draw_arrays(glow::TRIANGLES, 0, 3);
            self.gl.flush();
        }

        // Import GL FBO into wgpu texture
        let frame = self.producer.acquire_frame()?;
        let imported = self
            .importer
            .import_frame(&frame, &ImportOptions::default())?;

        // wgpu: present imported texture
        let surface_tex = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(tex)
            | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,
            other => return Err(format!("Failed to acquire surface texture: {other:?}").into()),
        };
        let surface_view = surface_tex
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let imported_view = imported
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&imported_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.blit_sampler),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.blit_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        surface_tex.present();
        Ok(())
    }
}
