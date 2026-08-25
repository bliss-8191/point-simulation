mod point_sim;
mod constant_input;

use std::sync::Arc;
use std::time::{Instant, Duration};
use std::collections::HashMap;

use pollster::FutureExt as _;

use winit::application::ApplicationHandler;
use winit::event::{WindowEvent, KeyEvent, ElementState};
use winit::keyboard::{Key, NamedKey};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId, Fullscreen};
use winit::dpi::PhysicalSize;
use winit::event::{Touch, TouchPhase};

use wgpu::{Extent3d, Instance, InstanceDescriptor, Texture, TextureDescriptor, TextureDimension};
use wgpu::RenderPass;
use wgpu::{PowerPreference, RequestAdapterOptions};
use wgpu::{Adapter, Device, Queue, DeviceDescriptor};
use wgpu::{Features, Limits, ExperimentalFeatures, MemoryHints, Trace};
use wgpu::{FeaturesWGPU, FeaturesWebGPU};
use wgpu::{Surface, SurfaceCapabilities, SurfaceConfiguration, CurrentSurfaceTexture, SurfaceTexture};
use wgpu::{TextureUsages, TextureFormat, SurfaceColorSpace, PresentMode, CompositeAlphaMode, TextureViewDescriptor, TextureAspect};
use wgpu::CommandEncoderDescriptor;

use point_sim::{PointPosition, PointSimBuilder, PointSim, PointSimDrawInfo, PointSimDraw};

pub(crate) struct CommonWgpuObjects {
    pub window: Arc<Window>,
    pub instance: Instance,
    pub adapter: Adapter,
    pub device: Device,
    pub queue: Queue,
    pub surface: Surface<'static>,
    pub surface_format: TextureFormat,
    pub surface_capabilities: SurfaceCapabilities,
}

const DEPTH_TEXTURE_FORMAT: TextureFormat = TextureFormat::Depth16Unorm;

struct FramerateController {
    target: Duration,
    current: Instant,
}
impl FramerateController {
    fn new(target_fps: f64) -> Self {
        let target = Duration::from_secs_f64(1.0 / target_fps);
        Self {
            target,
            current: Instant::now(),
        }
    }
    fn start_frame(&mut self) -> f64 {
        let frame_time_so_far = (Instant::now() - self.current).as_secs_f64();
        let time_to_sleep = self.target.as_secs_f64() - frame_time_so_far;
        if time_to_sleep > 0.0 {
            std::thread::sleep(Duration::from_secs_f64(time_to_sleep));
        }

        let current = Instant::now();
        let dt = (current - self.current).as_secs_f64();
        self.current = current;

        return dt;
    }
    fn set_framerate(&mut self, target_fps: f64) {
        self.target = Duration::from_secs_f64(1.0 / target_fps);
    }
}

struct App {
    constants: constant_input::Constants,

    co: CommonWgpuObjects,
    framerate_controller: FramerateController,

    depth_texture: Option<Texture>,

    mouse_pos: [f32; 2],
    mouse_pressed: bool,
    touch_points: HashMap<u64, [f32; 2]>,

    point_sim: Option<PointSim>,
    point_sim_draw: Option<PointSimDraw>,
}
impl App {
    fn create_point_sim(co: &CommonWgpuObjects, constants: &constant_input::Constants) -> (PointSim, PointSimDraw) {
        let mut initial_positions = Vec::<PointPosition>::new();
        let count = constants.point_count;
        for i in 0..count {
            initial_positions.push([0.0, 2.0 * i as f32 / count as f32 - 1.0]);
        }

        let builder = PointSimBuilder::new(&co)
            .initial_point_positions(&initial_positions)
            .update_rate(constants.update_rate)
            .constants(point_sim::SimulationConstants {
                input_force: constants.input_force,
                decay_factor: constants.decay_factor,
                target_radius: constants.target_radius,
                force_falloff: constants.force_falloff,
            });
        let point_sim = PointSim::from(builder);

        let point_sim_draw = PointSimDraw::new(
            &co, DEPTH_TEXTURE_FORMAT,
            &point_sim::DrawConstants {
                point_size: constants.point_size,
                corner_colors: constants.point_corner_colors,
                points_circular: constants.points_circular,
            },
        );

        (point_sim, point_sim_draw)
    }
    fn new(active_event_loop: &ActiveEventLoop) -> Self {
        let constants = constant_input::Constants::new();

        let fullscreen = if constants.fullscreen {
            Some(Fullscreen::Borderless(None))
        } else { None };

        let window = active_event_loop.create_window(
            Window::default_attributes()
            .with_title("Point Simulation")
            .with_visible(false)
            .with_fullscreen(fullscreen)
            .with_transparent(constants.transparent_window)
            ).unwrap();
        let window = Arc::new(window);
        let instance_descriptor = InstanceDescriptor::new_with_display_handle_from_env(Box::new(window.clone()));
        let instance = Instance::new(instance_descriptor);
        let surface = instance.create_surface(Box::new(window.clone())).unwrap();
        let adapter = instance.request_adapter(
            &RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            }
        ).block_on().unwrap();
        let surface_capabilities = surface.get_capabilities(&adapter);
        let mut limits = Limits::downlevel_defaults();
        // allow for largest possible window
        limits.max_texture_dimension_2d = adapter.limits().max_texture_dimension_2d;
        // (try to) make sure window can't go past that
        window.set_max_inner_size(Some(PhysicalSize::new(limits.max_texture_dimension_2d, limits.max_texture_dimension_2d)));
        // maximize limits that determine number of buffers required for simulation
        limits.max_storage_buffer_binding_size = adapter.limits().max_storage_buffer_binding_size;
        limits.max_compute_workgroups_per_dimension = adapter.limits().max_compute_workgroups_per_dimension;
        limits.max_buffer_size = adapter.limits().max_buffer_size;

        let (device, queue) = adapter.request_device(
            &DeviceDescriptor {
                label: None,
                required_features: Features {
                    features_wgpu: FeaturesWGPU::empty(),
                    features_webgpu: FeaturesWebGPU::empty(),
                },
                required_limits: limits,
                experimental_features: ExperimentalFeatures::disabled(),
                memory_hints: MemoryHints::MemoryUsage,
                trace: Trace::Off,
            }
        ).block_on().unwrap();

        let co = CommonWgpuObjects {
            window,
            instance,
            adapter,
            device,
            queue,
            surface,
            // unused default (always overwritten in configure_surface)
            surface_format: surface_capabilities.formats[0], 
            surface_capabilities,
        };

        let framerate_controller = FramerateController::new(Self::get_refresh(&co.window) as f64 / 1000.0);

        let (point_sim, point_sim_draw) = Self::create_point_sim(&co, &constants);

        co.window.set_visible(true);
        let mut rtn = Self {
            constants,

            co,
            framerate_controller,

            depth_texture: None,

            mouse_pos: [0.0, 0.0],
            mouse_pressed: false,
            touch_points: HashMap::with_capacity(point_sim::MAX_INPUT_POINTS.into()),

            point_sim: Some(point_sim),
            point_sim_draw: Some(point_sim_draw),
        };

        rtn.configure_surface();

        return rtn;
    }
    fn configure_surface(&mut self) -> TextureFormat {
        let co = &mut self.co;
        let size = {
            let mut size = co.window.inner_size();
            if size.width < 1 { size.width = 1 };
            if size.height < 1 { size.height = 1 };
            size
        };
        co.surface_format = co.surface_capabilities.formats[0];
        let mut alpha_mode = CompositeAlphaMode::Auto;
        if self.constants.transparent_window {
            for mode in &co.surface_capabilities.alpha_modes {
                if *mode == CompositeAlphaMode::PreMultiplied ||
                    *mode == CompositeAlphaMode::PostMultiplied
                {
                    alpha_mode = *mode;
                }
            }
        }
        let surface_configuration = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: co.surface_format,
            color_space: SurfaceColorSpace::Auto,
            width: size.width,
            height: size.height,
            present_mode: PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: alpha_mode,
            view_formats: vec![co.surface_format],
        };
        co.surface.configure(&co.device, &surface_configuration);

        self.depth_texture = None;
        self.depth_texture = Some(co.device.create_texture(&TextureDescriptor {
            label: None,
            size: Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: DEPTH_TEXTURE_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        }));

        return co.surface_format;
    }
    fn try_get_surface_texture(&mut self) -> Option<(SurfaceTexture, TextureFormat)> {
        let co = &mut self.co;
        match co.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(t) => return Some((t, co.surface_format)),
            CurrentSurfaceTexture::Occluded | CurrentSurfaceTexture::Timeout => return None,
            CurrentSurfaceTexture::Suboptimal(t) => {
                drop(t);
                self.configure_surface();
                return None;
            },
            CurrentSurfaceTexture::Outdated => {
                self.configure_surface();
                return None;
            },
            CurrentSurfaceTexture::Validation => {
                unreachable!();
            },
            CurrentSurfaceTexture::Lost => {
                co.surface = co.instance.create_surface(co.window.clone()).unwrap();
                co.surface_capabilities = co.surface.get_capabilities(&co.adapter);
                self.configure_surface();
                return None;
            }
        }
    }
    fn request_redraw(&self) {
        self.co.window.request_redraw();
    }
    fn get_refresh(window: &Window) -> u32 {
        if let Some(m)=window.current_monitor() {
            if let Some(r)=m.refresh_rate_millihertz() {
                return r;
            }
        }
        // fallback to 60hz if winit couldn't figure out refresh rate
        60_000
    }
    fn transform_input_position(window_size: &[f32; 2], pos: &[f32; 2]) -> [f32; 2] {
        if window_size[0] > window_size[1] { // width > height
            [
                (2.0 * pos[0] / window_size[0] - 1.0) * window_size[0] / window_size[1],
                -(2.0 * pos[1] / window_size[1] - 1.0),
            ]
        } else { // height > width
            [
                (2.0 * pos[0] / window_size[0] - 1.0),
                -(2.0 * pos[1] / window_size[1] - 1.0) * window_size[1] / window_size[0],
            ]
        }
    }
    fn draw(&mut self) {
        let refresh = Self::get_refresh(&self.co.window) as f64 / 1000.0;
        self.framerate_controller.set_framerate(refresh);
        let dt = self.framerate_controller.start_frame();

        let (surface_texture, surface_format) = if let Some(v)=self.try_get_surface_texture() { v } else { return; };
        let depth_texture = if let Some(t)=self.depth_texture.as_ref() { t } else { return; };
        let surface_texture_view = surface_texture.texture.create_view(&TextureViewDescriptor {
            label: None,
            format: Some(surface_format),
            dimension: None,
            usage: None,
            aspect: TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: 0,
            array_layer_count: None,
        });
        let depth_texture_view = depth_texture.create_view(&TextureViewDescriptor::default());

        let window_size = {
            let mut size = self.co.window.inner_size();
            if size.width < 1 { size.width = 1 };
            if size.height < 1 { size.height = 1 };
            [size.width as f32, size.height as f32]
        };
        // assemble all input positions
        let mut inputs = Vec::new();
        if self.mouse_pressed {
            inputs.push(Self::transform_input_position(&window_size, &self.mouse_pos));
        }
        for (_, pos) in self.touch_points.iter() {
            inputs.push(Self::transform_input_position(&window_size, &pos));
        }

        let point_sim = self.point_sim.as_mut().unwrap();
        let point_sim_draw = self.point_sim_draw.as_ref().unwrap();

        {
            let mut command_encoder = self.co.device.create_command_encoder(&CommandEncoderDescriptor { label: None });

            // bound simulation time delta to N intended frames
            // this avoids potential runaway behavior (simulation compute is proportional to dt)
            let simulation_dt = dt.min(self.constants.max_simulation_time_per_update / refresh);
            // run simulation update (updates "front" according to "back")
            // also generates geometry according to "back"
            point_sim.record_update(&self.co, &mut command_encoder, simulation_dt, &inputs);

            // bridge between simulation and rendering
            let draw_info = PointSimDrawInfo {
                window_size: &window_size,
                clear_color: &self.constants.clear_color,

                co: &self.co,
                command_encoder: &mut command_encoder,
                surface_texture_view: &surface_texture_view,
                depth_texture_view: &depth_texture_view,
                index_buffer_binder: |render_pass: &mut RenderPass| {
                    point_sim.bind_index_buffer(render_pass);
                },
                vertex_buffer_binders: point_sim.vertex_buffer_binders_iter(),
            };
            // record actual draw
            point_sim_draw.record_draw(draw_info);

            self.co.queue.submit([command_encoder.finish()]);
        }
        point_sim.swap_buffers();


        self.co.window.pre_present_notify();
        self.co.queue.present(surface_texture);
    }
    fn resized_event(&mut self) {
        self.configure_surface();
    }
    fn unhandled_event(&mut self, _active_event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) -> bool {
        match event {
            WindowEvent::KeyboardInput {event: key_event, ..} => match key_event {
                KeyEvent { logical_key, state: ElementState::Pressed, .. } => {
                    match logical_key {
                        Key::Named(NamedKey::Escape) => return false,
                        Key::Character(c) => {
                            match c.as_ref() {
                                "r" => {
                                    // delete simulation and draw first
                                    drop(self.point_sim.take());
                                    drop(self.point_sim_draw.take());

                                    self.constants = constant_input::Constants::new();

                                    let (point_sim, point_sim_draw) = Self::create_point_sim(&self.co, &self.constants);
                                    self.point_sim = Some(point_sim);
                                    self.point_sim_draw = Some(point_sim_draw);
                                    self.configure_surface();
                                },
                                _ => {},
                            }
                        },
                        _ => {},
                    }
                },
                _ => {},
            },
            WindowEvent::MouseInput { state: ElementState::Pressed, .. } => {
                self.mouse_pressed = true;
            },
            WindowEvent::MouseInput { state: ElementState::Released, .. } => {
                self.mouse_pressed = false;
            },
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_pos = [position.x as f32, position.y as f32];
            },
            WindowEvent::Touch(touch) => {
                match touch {
                    Touch { phase: TouchPhase::Started, location, id, .. } => {
                        _ = self.touch_points.insert(id, [location.x as f32, location.y as f32]);
                    },
                    Touch { phase: TouchPhase::Moved, location, id, .. } => {
                        _ = self.touch_points.insert(id, [location.x as f32, location.y as f32]);
                    },
                    Touch { phase: TouchPhase::Ended, id, .. } => {
                        _ = self.touch_points.remove(&id);
                    },
                    _ => {},
                }
            },
            _ => {},
        }
        return true
    }
}

#[derive(Default)]
struct WinitApp {
    app: Option<App>,
}

impl<'a> ApplicationHandler for WinitApp {
    fn resumed(&mut self, active_event_loop: &ActiveEventLoop) {
        self.app = Some(App::new(active_event_loop));
    }
    fn window_event(&mut self, active_event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                active_event_loop.exit();
            },
            WindowEvent::RedrawRequested => {
                let app = self.app.as_mut().unwrap();
                app.draw();
                app.request_redraw();
            },
            WindowEvent::Resized(_) => {
                let app = self.app.as_mut().unwrap();
                app.resized_event();
            },
            e => if !self.app.as_mut().unwrap().unhandled_event(active_event_loop, id, e) {
                active_event_loop.exit();
            }
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut winit_app = WinitApp::default();
    event_loop.run_app(&mut winit_app).unwrap();
}
