//! A geometric 3D view of the actual stitch command stream.
//!
//! Thread tubes are instanced on the GPU: the million-command limit does not
//! require a million copies of the mesh. The cloth and tubes render into a
//! private colour/depth target, then compose into egui's existing viewport.

use eframe::{egui, egui_glow, glow};
use embroidery_lab::{Command, Stitch};
use glow::HasContext as _;
use std::sync::{Arc, Mutex};

const FLOATS_PER_INSTANCE: usize = 12;
const MAX_SHADOW_STEPS: usize = 4_000_000;

#[derive(Clone, Copy)]
struct Camera {
    yaw: f32,
    elevation: f32,
    zoom: f32,
    pan: egui::Vec2,
}

#[derive(Clone, Copy)]
struct CameraDrag {
    button: egui::PointerButton,
    last_position: egui::Pos2,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.16,
            elevation: 1.08,
            zoom: 1.,
            pan: egui::Vec2::ZERO,
        }
    }
}

#[derive(Default)]
struct Scene {
    revision: u64,
    stitches: Vec<f32>,
    jumps: Vec<f32>,
    stitch_commands: Vec<usize>,
    jump_commands: Vec<usize>,
    width: f32,
    height: f32,
    board: [f32; 2],
    shadow: Vec<u8>,
    shadow_size: [i32; 2],
}

pub(super) struct Preview3d {
    gpu: Option<Arc<Mutex<Gpu>>>,
    scene: Arc<Scene>,
    revision: Option<u64>,
    camera: Camera,
    drag: Option<CameraDrag>,
    navigation_frame: Option<u64>,
    pub thread_diameter_mm: f32,
    source_error: Option<String>,
}

pub(super) struct PreviewLabels<'a> {
    pub empty: &'a str,
    pub stale: &'a str,
    pub unavailable: &'a str,
    pub navigation: &'a str,
}

impl Default for Preview3d {
    fn default() -> Self {
        Self {
            gpu: None,
            scene: Arc::new(Scene::default()),
            revision: None,
            camera: Camera::default(),
            drag: None,
            navigation_frame: None,
            thread_diameter_mm: 0.30,
            source_error: None,
        }
    }
}

impl Preview3d {
    pub fn new(gl: Option<Arc<glow::Context>>) -> Self {
        Self {
            gpu: gl.map(|gl| Arc::new(Mutex::new(Gpu::new(gl)))),
            ..Default::default()
        }
    }

    pub fn set_stitches(&mut self, stitches: &[Stitch], revision: u64) {
        if self.revision == Some(revision) {
            return;
        }
        self.revision = Some(revision);
        match build_scene(stitches, revision) {
            Ok(scene) => {
                self.scene = Arc::new(scene);
                self.source_error = None;
                self.reset_view();
            }
            Err(error) => {
                self.scene = Arc::new(Scene::default());
                self.source_error = Some(error);
            }
        }
    }

    pub fn reset_view(&mut self) {
        self.camera = Camera::default();
        self.drag = None;
    }

    pub fn error(&self) -> Option<String> {
        self.source_error.clone().or_else(|| {
            if self.gpu.is_none() {
                Some("3D preview requires the OpenGL renderer.".into())
            } else {
                self.gpu
                    .as_ref()
                    .and_then(|gpu| gpu.lock().ok()?.error.clone())
            }
        })
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        progress: usize,
        stale: bool,
        show_jumps: bool,
        labels: PreviewLabels<'_>,
    ) {
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0., egui::Color32::from_rgb(248, 249, 251));
        if stale || self.scene.stitch_commands.is_empty() {
            self.drag = None;
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if stale { labels.stale } else { labels.empty },
                egui::FontId::proportional(15.),
                egui::Color32::from_rgb(70, 80, 95),
            );
            return;
        }
        self.navigate(ui, &response);
        let Some(gpu) = self.gpu.clone() else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                labels.unavailable,
                egui::FontId::proportional(15.),
                egui::Color32::from_rgb(161, 43, 50),
            );
            return;
        };
        let scene = self.scene.clone();
        let context = ui.ctx().clone();
        let camera = self.camera;
        let diameter = if self.thread_diameter_mm.is_finite() {
            self.thread_diameter_mm.clamp(0.08, 0.8)
        } else {
            0.30
        };
        painter.add(egui::PaintCallback {
            rect,
            callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                if let Ok(mut gpu) = gpu.lock() {
                    let had_error = gpu.error.is_some();
                    gpu.paint(
                        &info,
                        painter.intermediate_fbo(),
                        &scene,
                        RenderSettings {
                            camera,
                            diameter,
                            progress,
                            show_jumps,
                        },
                    );
                    if !had_error && gpu.error.is_some() {
                        context.request_repaint();
                    }
                }
            })),
        });
        painter.text(
            rect.left_bottom() + egui::vec2(14., -14.),
            egui::Align2::LEFT_BOTTOM,
            labels.navigation,
            egui::FontId::proportional(12.),
            egui::Color32::from_rgb(70, 80, 95),
        );
    }

    /// Track the ordered pointer events, rather than only the button state at
    /// frame end. A quick press/move/release can arrive together after an
    /// expensive GPU frame; it must still rotate or pan exactly once.
    fn navigate(&mut self, ui: &egui::Ui, response: &egui::Response) {
        let frame = ui.ctx().cumulative_frame_nr();
        if self.navigation_frame == Some(frame) {
            return;
        }
        self.navigation_frame = Some(frame);
        let (events, focused) = ui.input(|input| (input.events.clone(), input.raw.focused));
        if !focused || !ui.is_enabled() {
            self.drag = None;
            return;
        }
        for event in events {
            match event {
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    ..
                } if matches!(
                    button,
                    egui::PointerButton::Primary
                        | egui::PointerButton::Middle
                        | egui::PointerButton::Secondary
                ) =>
                {
                    self.drag = None;
                    let unobstructed = ui
                        .ctx()
                        .layer_id_at(pos)
                        .is_none_or(|layer| layer == response.layer_id);
                    if response.rect.contains(pos) && unobstructed {
                        self.drag = Some(CameraDrag {
                            button,
                            last_position: pos,
                        });
                    }
                }
                egui::Event::PointerMoved(pos) => self.move_drag(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: false,
                    ..
                } => {
                    if self.drag.is_some_and(|drag| drag.button == button) {
                        self.move_drag(pos);
                        self.drag = None;
                    }
                }
                egui::Event::PointerGone => self.drag = None,
                _ => {}
            }
        }
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0. {
                self.camera.zoom = (self.camera.zoom * (scroll * 0.002).exp()).clamp(0.15, 25.);
            }
        }
        if response.double_clicked() {
            self.reset_view();
        }
    }

    fn move_drag(&mut self, position: egui::Pos2) {
        let Some(drag) = &mut self.drag else {
            return;
        };
        let delta = position - drag.last_position;
        drag.last_position = position;
        if !delta.is_finite() {
            self.drag = None;
            return;
        }
        match drag.button {
            egui::PointerButton::Primary => {
                self.camera.yaw -= delta.x * 0.007;
                self.camera.elevation = (self.camera.elevation + delta.y * 0.007).clamp(0.12, 1.55);
            }
            egui::PointerButton::Middle | egui::PointerButton::Secondary => {
                self.camera.pan += delta
            }
            _ => {}
        }
    }

    /// Call while eframe's GL context is still current (App::on_exit).
    pub fn destroy(&mut self) {
        if let Some(gpu) = self.gpu.take()
            && let Ok(mut gpu) = gpu.lock()
        {
            gpu.destroy();
        }
    }
}

fn positional_segments(stitches: &[Stitch]) -> Result<Vec<(usize, Stitch, [f64; 2])>, String> {
    let mut segments = Vec::new();
    let mut previous = [0., 0.];
    for (index, stitch) in stitches.iter().copied().enumerate() {
        match stitch.command {
            Command::End => break,
            Command::ColorChange | Command::Jump | Command::Stitch => {
                if !stitch.x.is_finite() || !stitch.y.is_finite() {
                    return Err("3D preview cannot display non-finite stitch coordinates.".into());
                }
                if stitch.command != Command::ColorChange && previous != [stitch.x, stitch.y] {
                    segments.push((index, stitch, previous));
                }
                previous = [stitch.x, stitch.y];
            }
        }
    }
    Ok(segments)
}

fn build_scene(stitches: &[Stitch], revision: u64) -> Result<Scene, String> {
    let segments = positional_segments(stitches)?;
    let sewn: Vec<_> = segments
        .iter()
        .filter(|(_, stitch, _)| stitch.command == Command::Stitch)
        .collect();
    if sewn.is_empty() {
        return Ok(Scene {
            revision,
            ..Default::default()
        });
    }
    let mut minimum = [f64::INFINITY; 2];
    let mut maximum = [f64::NEG_INFINITY; 2];
    for (_, stitch, start) in &sewn {
        for point in [*start, [stitch.x, stitch.y]] {
            for axis in 0..2 {
                minimum[axis] = minimum[axis].min(point[axis]);
                maximum[axis] = maximum[axis].max(point[axis]);
            }
        }
    }
    let center = [
        (minimum[0] + maximum[0]) * 0.5,
        (minimum[1] + maximum[1]) * 0.5,
    ];
    let width = (maximum[0] - minimum[0]) as f32;
    let height = (maximum[1] - minimum[1]) as f32;
    if !width.is_finite() || !height.is_finite() {
        return Err("3D preview coordinates exceed the supported range.".into());
    }
    let margin = width.max(height).mul_add(0.08, 4.).max(5.);
    let board = [width.max(1.) + margin * 2., height.max(1.) + margin * 2.];
    let longest = board[0].max(board[1]);
    let shadow_size = [
        ((board[0] / longest * 768.).ceil() as i32).max(2),
        ((board[1] / longest * 768.).ceil() as i32).max(2),
    ];
    let cells = shadow_size[0] as usize * shadow_size[1] as usize;
    let mut shadow = vec![0_u8; cells];
    let mut owners = vec![None; cells];
    let mut heights = vec![0_u8; cells];
    let max_steps = (MAX_SHADOW_STEPS / sewn.len()).clamp(1, 32);
    let mut scene = Scene {
        revision,
        stitches: Vec::with_capacity(sewn.len() * FLOATS_PER_INSTANCE),
        width,
        height,
        board,
        shadow_size,
        ..Default::default()
    };
    let cell_index = |x: f32, y: f32| {
        let sx = ((x / board[0] + 0.5) * shadow_size[0] as f32).floor() as i32;
        let sy = ((y / board[1] + 0.5) * shadow_size[1] as f32).floor() as i32;
        let sx = sx.clamp(0, shadow_size[0] - 1) as usize;
        let sy = sy.clamp(0, shadow_size[1] - 1) as usize;
        sy * shadow_size[0] as usize + sx
    };
    let cell_mm = longest / 768.;
    for (index, stitch, start) in segments {
        let a = [
            (start[0] - center[0]) as f32,
            -(start[1] - center[1]) as f32,
        ];
        let b = [
            (stitch.x - center[0]) as f32,
            -(stitch.y - center[1]) as f32,
        ];
        let length = (b[0] - a[0]).hypot(b[1] - a[1]);
        let jump = stitch.command == Command::Jump;
        let mut layer = 0;
        if !jump {
            let steps = ((length / cell_mm).ceil() as usize).clamp(1, max_steps);
            for step in 0..=steps {
                let fraction = step as f32 / steps as f32;
                let cell = cell_index(
                    a[0] + (b[0] - a[0]) * fraction,
                    a[1] + (b[1] - a[1]) * fraction,
                );
                if owners[cell].is_some_and(|key| key != (stitch.object_id, stitch.color)) {
                    layer = layer.max(heights[cell].saturating_add(1).min(5));
                } else {
                    layer = layer.max(heights[cell]);
                }
            }
            for step in 0..=steps {
                let fraction = step as f32 / steps as f32;
                let cell = cell_index(
                    a[0] + (b[0] - a[0]) * fraction,
                    a[1] + (b[1] - a[1]) * fraction,
                );
                owners[cell] = Some((stitch.object_id, stitch.color));
                heights[cell] = layer;
                shadow[cell] = 255;
            }
        }
        let color = if jump { [195, 90, 64] } else { stitch.color };
        let target = if jump {
            &mut scene.jumps
        } else {
            &mut scene.stitches
        };
        target.extend_from_slice(&[
            a[0],
            a[1],
            f32::from(layer),
            0.,
            b[0],
            b[1],
            length,
            0.,
            f32::from(color[0]) / 255.,
            f32::from(color[1]) / 255.,
            f32::from(color[2]) / 255.,
            if jump { 1. } else { 0. },
        ]);
        if jump {
            scene.jump_commands.push(index);
        } else {
            scene.stitch_commands.push(index);
        }
    }
    scene.shadow = shadow;
    Ok(scene)
}

struct Resources {
    program: glow::Program,
    tube_vao: glow::VertexArray,
    tube_vertices: glow::Buffer,
    tube_indices: glow::Buffer,
    instances: glow::Buffer,
    jump_instances: glow::Buffer,
    cloth_vao: glow::VertexArray,
    cloth_vertices: glow::Buffer,
    shadow_texture: glow::Texture,
    framebuffer: glow::Framebuffer,
    color_texture: glow::Texture,
    depth_buffer: glow::Renderbuffer,
    target_size: [i32; 2],
    revision: Option<u64>,
    index_count: i32,
}

struct Gpu {
    gl: Arc<glow::Context>,
    resources: Option<Resources>,
    error: Option<String>,
}

#[derive(Clone, Copy)]
struct RenderSettings {
    camera: Camera,
    diameter: f32,
    progress: usize,
    show_jumps: bool,
}

/// Restore shared GL state even when resource allocation or framebuffer setup
/// fails before the first rendered frame.
struct GlState {
    gl: Arc<glow::Context>,
    viewport: [i32; 4],
    scissor_box: [i32; 4],
    enabled: [bool; 4],
    depth_mask: bool,
    depth_func: u32,
    read_framebuffer: Option<glow::Framebuffer>,
    draw_framebuffer: Option<glow::Framebuffer>,
    program: Option<glow::Program>,
    vertex_array: Option<glow::VertexArray>,
    array_buffer: Option<glow::Buffer>,
    renderbuffer: Option<glow::Renderbuffer>,
    active_texture: u32,
    texture_zero: Option<glow::Texture>,
    unpack_alignment: i32,
}

impl GlState {
    unsafe fn capture(gl: Arc<glow::Context>) -> Self {
        unsafe {
            let mut viewport = [0; 4];
            let mut scissor_box = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            gl.get_parameter_i32_slice(glow::SCISSOR_BOX, &mut scissor_box);
            let active_texture = gl.get_parameter_i32(glow::ACTIVE_TEXTURE) as u32;
            gl.active_texture(glow::TEXTURE0);
            let texture_zero = gl.get_parameter_texture(glow::TEXTURE_BINDING_2D);
            gl.active_texture(active_texture);
            Self {
                viewport,
                scissor_box,
                enabled: [
                    gl.is_enabled(glow::SCISSOR_TEST),
                    gl.is_enabled(glow::BLEND),
                    gl.is_enabled(glow::DEPTH_TEST),
                    gl.is_enabled(glow::CULL_FACE),
                ],
                depth_mask: gl.get_parameter_bool(glow::DEPTH_WRITEMASK),
                depth_func: gl.get_parameter_i32(glow::DEPTH_FUNC) as u32,
                read_framebuffer: gl.get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING),
                draw_framebuffer: gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING),
                program: gl.get_parameter_program(glow::CURRENT_PROGRAM),
                vertex_array: gl.get_parameter_vertex_array(glow::VERTEX_ARRAY_BINDING),
                array_buffer: gl.get_parameter_buffer(glow::ARRAY_BUFFER_BINDING),
                renderbuffer: gl.get_parameter_renderbuffer(glow::RENDERBUFFER_BINDING),
                unpack_alignment: gl.get_parameter_i32(glow::UNPACK_ALIGNMENT),
                active_texture,
                texture_zero,
                gl,
            }
        }
    }
}

impl Drop for GlState {
    fn drop(&mut self) {
        let gl = &self.gl;
        unsafe {
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, self.read_framebuffer);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, self.draw_framebuffer);
            gl.bind_renderbuffer(glow::RENDERBUFFER, self.renderbuffer);
            gl.bind_vertex_array(self.vertex_array);
            gl.bind_buffer(glow::ARRAY_BUFFER, self.array_buffer);
            gl.use_program(self.program);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, self.texture_zero);
            gl.active_texture(self.active_texture);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, self.unpack_alignment);
            gl.depth_mask(self.depth_mask);
            gl.depth_func(self.depth_func);
            for (capability, enabled) in [
                glow::SCISSOR_TEST,
                glow::BLEND,
                glow::DEPTH_TEST,
                glow::CULL_FACE,
            ]
            .into_iter()
            .zip(self.enabled)
            {
                set_enabled(gl, capability, enabled);
            }
            gl.viewport(
                self.viewport[0],
                self.viewport[1],
                self.viewport[2],
                self.viewport[3],
            );
            gl.scissor(
                self.scissor_box[0],
                self.scissor_box[1],
                self.scissor_box[2],
                self.scissor_box[3],
            );
        }
    }
}

impl Gpu {
    fn new(gl: Arc<glow::Context>) -> Self {
        Self {
            gl,
            resources: None,
            error: None,
        }
    }

    fn paint(
        &mut self,
        info: &egui::PaintCallbackInfo,
        destination: Option<glow::Framebuffer>,
        scene: &Scene,
        settings: RenderSettings,
    ) {
        if self.error.is_some() {
            return;
        }
        let version = self.gl.version();
        if version.is_embedded || version.major < 3 || (version.major == 3 && version.minor < 3) {
            self.error = Some("3D preview requires OpenGL 3.3 or newer.".into());
            return;
        }
        let _state = unsafe { GlState::capture(self.gl.clone()) };
        let RenderSettings {
            camera,
            diameter,
            progress,
            show_jumps,
        } = settings;
        if self.resources.is_none() {
            match unsafe { Resources::new(&self.gl) } {
                Ok(resources) => self.resources = Some(resources),
                Err(error) => {
                    self.error = Some(format!("3D preview could not initialize OpenGL: {error}"));
                    return;
                }
            }
        }
        let gl = &self.gl;
        let resources = self.resources.as_mut().unwrap();
        let viewport = info.viewport_in_pixels();
        if viewport.width_px <= 0 || viewport.height_px <= 0 {
            return;
        }
        unsafe {
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LESS);
            gl.depth_mask(true);
            if resources.revision != Some(scene.revision) {
                resources.upload(gl, scene);
            }
            let size = [viewport.width_px, viewport.height_px];
            if let Err(error) = resources.resize(gl, size) {
                self.error = Some(error);
                return;
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(resources.framebuffer));
            gl.viewport(0, 0, size[0], size[1]);
            gl.clear_color(248. / 255., 249. / 255., 251. / 255., 1.);
            gl.clear_depth_f32(1.);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            gl.use_program(Some(resources.program));
            let aspect = size[0] as f32 / size[1] as f32;
            let radius = (scene.width / aspect).max(scene.height).max(10.) * 0.5 + 4.;
            let distance = radius / (35_f32.to_radians() * 0.5).tan() * 1.18 / camera.zoom;
            let horizontal = camera.elevation.cos() * distance;
            let eye = [
                horizontal * camera.yaw.sin(),
                -horizontal * camera.yaw.cos(),
                camera.elevation.sin() * distance,
            ];
            let mut projection = perspective(
                35_f32.to_radians(),
                aspect,
                (distance / 500.).max(0.01),
                distance * 5. + scene.board[0].max(scene.board[1]) * 2.,
            );
            projection[8] -= camera.pan.x * 2. / info.viewport.width();
            projection[9] += camera.pan.y * 2. / info.viewport.height();
            let matrix = multiply(projection, look_at(eye));
            gl.uniform_matrix_4_f32_slice(
                gl.get_uniform_location(resources.program, "u_matrix")
                    .as_ref(),
                false,
                &matrix,
            );
            gl.uniform_3_f32(
                gl.get_uniform_location(resources.program, "u_eye").as_ref(),
                eye[0],
                eye[1],
                eye[2],
            );
            gl.uniform_2_f32(
                gl.get_uniform_location(resources.program, "u_board")
                    .as_ref(),
                scene.board[0],
                scene.board[1],
            );
            gl.uniform_1_f32(
                gl.get_uniform_location(resources.program, "u_diameter")
                    .as_ref(),
                diameter,
            );
            gl.uniform_1_i32(
                gl.get_uniform_location(resources.program, "u_shadow")
                    .as_ref(),
                0,
            );
            let sewn_count = scene
                .stitch_commands
                .partition_point(|index| *index < progress);
            gl.uniform_1_f32(
                gl.get_uniform_location(resources.program, "u_shadow_weight")
                    .as_ref(),
                if sewn_count == scene.stitch_commands.len() {
                    1.
                } else {
                    0.
                },
            );
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(resources.shadow_texture));
            gl.uniform_1_i32(
                gl.get_uniform_location(resources.program, "u_mode")
                    .as_ref(),
                0,
            );
            gl.bind_vertex_array(Some(resources.cloth_vao));
            gl.draw_arrays(glow::TRIANGLES, 0, 6);
            gl.uniform_1_i32(
                gl.get_uniform_location(resources.program, "u_mode")
                    .as_ref(),
                1,
            );
            gl.bind_vertex_array(Some(resources.tube_vao));
            resources.bind_instances(gl, resources.instances);
            gl.draw_elements_instanced(
                glow::TRIANGLES,
                resources.index_count,
                glow::UNSIGNED_INT,
                0,
                sewn_count as i32,
            );
            if show_jumps {
                let count = scene
                    .jump_commands
                    .partition_point(|index| *index < progress);
                resources.bind_instances(gl, resources.jump_instances);
                gl.draw_elements_instanced(
                    glow::TRIANGLES,
                    resources.index_count,
                    glow::UNSIGNED_INT,
                    0,
                    count as i32,
                );
            }
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(resources.framebuffer));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, destination);
            gl.disable(glow::DEPTH_TEST);
            let clip = info.clip_rect_in_pixels();
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(
                clip.left_px,
                clip.from_bottom_px,
                clip.width_px,
                clip.height_px,
            );
            gl.blit_framebuffer(
                0,
                0,
                size[0],
                size[1],
                viewport.left_px,
                viewport.from_bottom_px,
                viewport.left_px + size[0],
                viewport.from_bottom_px + size[1],
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, destination);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }

    fn destroy(&mut self) {
        if let Some(resources) = self.resources.take() {
            unsafe {
                resources.destroy(&self.gl);
            }
        }
    }
}

unsafe fn set_enabled(gl: &glow::Context, capability: u32, enabled: bool) {
    unsafe {
        if enabled {
            gl.enable(capability);
        } else {
            gl.disable(capability);
        }
    }
}

fn bytes<T>(values: &[T]) -> &[u8] {
    // These callers only use initialized primitive f32/u32 arrays.
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast(), std::mem::size_of_val(values)) }
}

impl Resources {
    unsafe fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            let program = compile_program(gl)?;
            let tube_vao = gl.create_vertex_array()?;
            let tube_vertices = gl.create_buffer()?;
            let tube_indices = gl.create_buffer()?;
            let instances = gl.create_buffer()?;
            let jump_instances = gl.create_buffer()?;
            gl.bind_vertex_array(Some(tube_vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(tube_vertices));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(tube_indices));
            let cloth_vao = gl.create_vertex_array()?;
            let cloth_vertices = gl.create_buffer()?;
            gl.bind_vertex_array(Some(cloth_vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(cloth_vertices));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytes(&[0_f32, 0., 1., 0., 1., 1., 0., 0., 1., 1., 0., 1.]),
                glow::STATIC_DRAW,
            );
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            let shadow_texture = gl.create_texture()?;
            let color_texture = gl.create_texture()?;
            gl.active_texture(glow::TEXTURE0);
            for texture in [shadow_texture, color_texture] {
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::LINEAR as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::LINEAR as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_S,
                    glow::CLAMP_TO_EDGE as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_T,
                    glow::CLAMP_TO_EDGE as i32,
                );
            }
            let framebuffer = gl.create_framebuffer()?;
            let depth_buffer = gl.create_renderbuffer()?;
            gl.bind_vertex_array(None);
            Ok(Self {
                program,
                tube_vao,
                tube_vertices,
                tube_indices,
                instances,
                jump_instances,
                cloth_vao,
                cloth_vertices,
                shadow_texture,
                framebuffer,
                color_texture,
                depth_buffer,
                target_size: [0, 0],
                revision: None,
                index_count: 0,
            })
        }
    }

    unsafe fn bind_instances(&self, gl: &glow::Context, buffer: glow::Buffer) {
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
            for attribute in 1..=3 {
                gl.enable_vertex_attrib_array(attribute);
                gl.vertex_attrib_pointer_f32(
                    attribute,
                    4,
                    glow::FLOAT,
                    false,
                    48,
                    (attribute as i32 - 1) * 16,
                );
                gl.vertex_attrib_divisor(attribute, 1);
            }
        }
    }

    unsafe fn upload(&mut self, gl: &glow::Context, scene: &Scene) {
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.instances));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytes(&scene.stitches),
                glow::STATIC_DRAW,
            );
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.jump_instances));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&scene.jumps), glow::STATIC_DRAW);
            let (radial, arcs) = if scene.stitch_commands.len() > 150_000 {
                (4, 3)
            } else {
                (6, 5)
            };
            let (vertices, indices) = tube_mesh(radial, arcs);
            self.index_count = indices.len() as i32;
            gl.bind_vertex_array(Some(self.tube_vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.tube_vertices));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&vertices), glow::STATIC_DRAW);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.tube_indices));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                bytes(&indices),
                glow::STATIC_DRAW,
            );
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.shadow_texture));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::R8 as i32,
                scene.shadow_size[0],
                scene.shadow_size[1],
                0,
                glow::RED,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&scene.shadow)),
            );
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            self.revision = Some(scene.revision);
        }
    }

    unsafe fn resize(&mut self, gl: &glow::Context, size: [i32; 2]) -> Result<(), String> {
        if self.target_size == size {
            return Ok(());
        }
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.color_texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                size[0],
                size[1],
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(self.depth_buffer));
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::DEPTH_COMPONENT24,
                size[0],
                size[1],
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.framebuffer));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(self.color_texture),
                0,
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(self.depth_buffer),
            );
            if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                return Err("3D preview could not create its depth framebuffer.".into());
            }
            self.target_size = size;
            Ok(())
        }
    }

    unsafe fn destroy(self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            for vao in [self.tube_vao, self.cloth_vao] {
                gl.delete_vertex_array(vao);
            }
            for buffer in [
                self.tube_vertices,
                self.tube_indices,
                self.instances,
                self.jump_instances,
                self.cloth_vertices,
            ] {
                gl.delete_buffer(buffer);
            }
            for texture in [self.shadow_texture, self.color_texture] {
                gl.delete_texture(texture);
            }
            gl.delete_framebuffer(self.framebuffer);
            gl.delete_renderbuffer(self.depth_buffer);
        }
    }
}

fn tube_mesh(radial: usize, arcs: usize) -> (Vec<f32>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for segment in 0..=arcs {
        for side in 0..=radial {
            vertices.extend_from_slice(&[
                segment as f32 / arcs as f32,
                side as f32 / radial as f32 * std::f32::consts::TAU,
            ]);
        }
    }
    for segment in 0..arcs {
        for side in 0..radial {
            let a = (segment * (radial + 1) + side) as u32;
            let b = a + (radial + 1) as u32;
            indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    (vertices, indices)
}

unsafe fn compile_program(gl: &glow::Context) -> Result<glow::Program, String> {
    unsafe {
        let program = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, source) in [
            (glow::VERTEX_SHADER, VERTEX_SHADER),
            (glow::FRAGMENT_SHADER, FRAGMENT_SHADER),
        ] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let error = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                for shader in shaders {
                    gl.delete_shader(shader);
                }
                gl.delete_program(program);
                return Err(error);
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if !gl.get_program_link_status(program) {
            let error = gl.get_program_info_log(program);
            gl.delete_program(program);
            Err(error)
        } else {
            Ok(program)
        }
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    v.map(|component| component / length)
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn look_at(eye: [f32; 3]) -> [f32; 16] {
    let forward = normalize(eye.map(|component| -component));
    let right = normalize(cross(forward, [0., 0., 1.]));
    let up = cross(right, forward);
    [
        right[0],
        up[0],
        -forward[0],
        0.,
        right[1],
        up[1],
        -forward[1],
        0.,
        right[2],
        up[2],
        -forward[2],
        0.,
        -dot(right, eye),
        -dot(up, eye),
        dot(forward, eye),
        1.,
    ]
}
fn perspective(fov: f32, aspect: f32, near: f32, far: f32) -> [f32; 16] {
    let scale = 1. / (fov * 0.5).tan();
    [
        scale / aspect,
        0.,
        0.,
        0.,
        0.,
        scale,
        0.,
        0.,
        0.,
        0.,
        (far + near) / (near - far),
        -1.,
        0.,
        0.,
        2. * far * near / (near - far),
        0.,
    ]
}
fn multiply(a: [f32; 16], b: [f32; 16]) -> [f32; 16] {
    let mut result = [0.; 16];
    for column in 0..4 {
        for row in 0..4 {
            for k in 0..4 {
                result[column * 4 + row] += a[k * 4 + row] * b[column * 4 + k];
            }
        }
    }
    result
}

const VERTEX_SHADER: &str = r#"#version 330 core
layout(location=0) in vec2 a_mesh;
layout(location=1) in vec4 a_start;
layout(location=2) in vec4 a_end;
layout(location=3) in vec4 a_color;
uniform mat4 u_matrix;
uniform vec2 u_board;
uniform float u_diameter;
uniform int u_mode;
out vec3 v_world;
out vec3 v_normal;
out vec4 v_color;
out vec2 v_uv;
out float v_length;
void main() {
    if (u_mode == 0) {
        v_world=vec3((a_mesh-0.5)*u_board,-0.12);
        v_normal=vec3(0.0,0.0,1.0);
        v_color=vec4(1.0);
        v_uv=a_mesh;
        v_length=0.0;
    } else {
        float t=a_mesh.x;
        float angle=a_mesh.y;
        float jump=a_color.a;
        float radius=u_diameter*0.5*mix(1.0,0.15,jump);
        float s=sin(3.14159265*t);
        float taper=mix(0.35,1.0,pow(max(s,0.0),0.35));
        vec2 delta=a_end.xy-a_start.xy;
        float len=max(length(delta),0.00001);
        vec3 tangent=normalize(vec3(delta, radius*0.9*3.14159265*cos(3.14159265*t)));
        vec3 side=vec3(-delta.y/len,delta.x/len,0.0);
        vec3 up=normalize(cross(tangent,side));
        v_normal=normalize(side*sin(angle)+up*cos(angle));
        float height=radius+u_diameter*a_start.z*0.7+radius*0.9*s;
        height=mix(height,0.7+u_diameter*2.0,jump);
        v_world=vec3(mix(a_start.xy,a_end.xy,t),height)+v_normal*radius*taper;
        v_color=a_color;
        v_uv=vec2(t,angle);
        v_length=a_end.z;
    }
    gl_Position=u_matrix*vec4(v_world,1.0);
}
"#;

const FRAGMENT_SHADER: &str = r#"#version 330 core
in vec3 v_world;
in vec3 v_normal;
in vec4 v_color;
in vec2 v_uv;
in float v_length;
uniform int u_mode;
uniform vec3 u_eye;
uniform vec2 u_board;
uniform float u_diameter;
uniform sampler2D u_shadow;
uniform float u_shadow_weight;
out vec4 out_color;
void main() {
    vec3 light=normalize(vec3(-0.35,0.45,0.85));
    if(u_mode==0) {
        vec2 blur=vec2(max(u_diameter*0.7,0.12))/u_board;
        vec2 offset=vec2(0.18,-0.22)/u_board;
        float shadow=texture(u_shadow,v_uv+offset).r*0.35;
        shadow+=texture(u_shadow,v_uv+offset+vec2(blur.x,0.0)).r*0.16;
        shadow+=texture(u_shadow,v_uv+offset-vec2(blur.x,0.0)).r*0.16;
        shadow+=texture(u_shadow,v_uv+offset+vec2(0.0,blur.y)).r*0.16;
        shadow+=texture(u_shadow,v_uv+offset-vec2(0.0,blur.y)).r*0.16;
        float attenuation=1.0-smoothstep(0.25,0.8,max(fwidth(v_world.x),fwidth(v_world.y))*8.0);
        float weave=(sin(v_world.x*50.0)+sin(v_world.y*50.0))*0.007*attenuation;
        vec3 cloth=vec3(0.94,0.93,0.90)+weave;
        cloth*=1.0-shadow*0.29*u_shadow_weight;
        out_color=vec4(cloth,1.0);
    } else {
        if(v_color.a>0.5 && fract(v_uv.x*v_length/1.5)>0.58) discard;
        vec3 normal=normalize(v_normal);
        vec3 view=normalize(u_eye-v_world);
        vec3 half_vector=normalize(light+view);
        float diffuse=max(dot(normal,light),0.0);
        float sheen=pow(max(dot(normal,half_vector),0.0),38.0)*0.32;
        float fiber=sin(v_uv.y*12.0+v_uv.x*v_length*70.0)*0.012;
        vec3 color=v_color.rgb*(0.54+0.46*diffuse+fiber)+vec3(sheen);
        out_color=vec4(color,1.0);
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn stitch(command: Command, x: f64, y: f64) -> Stitch {
        Stitch {
            command,
            x,
            y,
            color: [80, 100, 200],
            object_id: 1,
        }
    }

    #[test]
    fn scene_uses_only_positional_commands_and_respects_end() {
        let data = vec![
            stitch(Command::Jump, 100., 200.),
            stitch(Command::Stitch, 110., 200.),
            stitch(Command::ColorChange, 110., 203.),
            stitch(Command::Stitch, 110., 205.),
            stitch(Command::End, 0., 0.),
            stitch(Command::Stitch, 500., 500.),
        ];
        let scene = build_scene(&data, 7).unwrap();
        assert_eq!(scene.stitch_commands, [1, 3]);
        assert_eq!(scene.jump_commands, [0]);
        assert_eq!(scene.width, 10.);
        assert_eq!(scene.height, 5.);
        assert_eq!(scene.stitches.len(), 2 * FLOATS_PER_INSTANCE);
        assert_eq!(scene.stitches[0], -5.);
        assert_eq!(scene.stitches[FLOATS_PER_INSTANCE], 5.);
        assert_eq!(scene.stitches[FLOATS_PER_INSTANCE + 1], -0.5);
    }

    #[test]
    fn simulation_prefix_and_camera_matrices_remain_finite() {
        let data = vec![
            stitch(Command::Jump, -10., -20.),
            stitch(Command::Stitch, 0., 0.),
            stitch(Command::Stitch, 10., 0.),
        ];
        let scene = build_scene(&data, 9).unwrap();
        assert_eq!(scene.stitch_commands.partition_point(|index| *index < 2), 1);
        for elevation in [0.12_f32, 1.08, 1.55] {
            let eye = [0., -100. * elevation.cos(), 100. * elevation.sin()];
            let matrix = multiply(perspective(0.6, 1.3, 0.1, 1000.), look_at(eye));
            assert!(matrix.iter().all(|value| value.is_finite()));
        }
    }

    #[test]
    fn meshes_are_indexed_and_have_finite_radial_parameters() {
        for (radial, arcs) in [(6, 5), (4, 3)] {
            let (vertices, indices) = tube_mesh(radial, arcs);
            assert_eq!(indices.len(), radial * arcs * 6);
            assert!(
                indices
                    .iter()
                    .all(|index| (*index as usize) < vertices.len() / 2)
            );
            assert!(vertices.iter().all(|value| value.is_finite()));
        }
    }

    #[test]
    fn no_stitches_and_invalid_geometry_are_explicit_states() {
        assert!(
            build_scene(&[stitch(Command::Jump, 1., 2.)], 1)
                .unwrap()
                .stitches
                .is_empty()
        );
        assert!(build_scene(&[stitch(Command::Stitch, f64::NAN, 1.)], 1).is_err());
    }

    fn navigation_frame(
        preview: &mut Preview3d,
        context: &egui::Context,
        events: Vec<egui::Event>,
    ) {
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900., 680.),
                )),
                focused: true,
                events,
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    preview.show(
                        ui,
                        usize::MAX,
                        false,
                        false,
                        PreviewLabels {
                            empty: "empty",
                            stale: "stale",
                            unavailable: "unavailable",
                            navigation: "navigation",
                        },
                    );
                });
            },
        );
    }

    fn pointer(position: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: position,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn navigation_preview() -> Preview3d {
        Preview3d {
            scene: Arc::new(build_scene(&[stitch(Command::Stitch, 10., 10.)], 1).unwrap()),
            ..Default::default()
        }
    }

    #[test]
    fn batched_pointer_drag_rotates_and_pans_without_capturing_an_outside_press() {
        let context = egui::Context::default();
        let mut preview = navigation_preview();
        navigation_frame(&mut preview, &context, vec![]);
        navigation_frame(&mut preview, &context, vec![]);
        let start = egui::pos2(300., 300.);
        let end = egui::pos2(380., 330.);
        navigation_frame(
            &mut preview,
            &context,
            vec![
                egui::Event::PointerMoved(start),
                pointer(start, egui::PointerButton::Primary, true),
                egui::Event::PointerMoved(end),
                pointer(end, egui::PointerButton::Primary, false),
            ],
        );
        assert!((preview.camera.yaw - (Camera::default().yaw - 80. * 0.007)).abs() < 1e-6);
        assert!(
            (preview.camera.elevation - (Camera::default().elevation + 30. * 0.007)).abs() < 1e-6
        );
        assert!(preview.drag.is_none());
        navigation_frame(
            &mut preview,
            &context,
            vec![
                egui::Event::PointerMoved(start),
                pointer(start, egui::PointerButton::Middle, true),
                egui::Event::PointerMoved(end),
                pointer(end, egui::PointerButton::Middle, false),
            ],
        );
        assert_eq!(preview.camera.pan, egui::vec2(80., 30.));
        let previous = preview.camera;
        let outside = egui::pos2(1., 1.);
        navigation_frame(
            &mut preview,
            &context,
            vec![
                egui::Event::PointerMoved(outside),
                pointer(outside, egui::PointerButton::Primary, true),
                egui::Event::PointerMoved(end),
                pointer(end, egui::PointerButton::Primary, false),
            ],
        );
        assert_eq!(preview.camera.yaw, previous.yaw);
        assert_eq!(preview.camera.elevation, previous.elevation);
        assert_eq!(preview.camera.pan, previous.pan);
        assert!(preview.drag.is_none());
    }

    #[test]
    fn multi_frame_drag_includes_final_release_motion_once() {
        let context = egui::Context::default();
        let mut preview = navigation_preview();
        navigation_frame(&mut preview, &context, vec![]);
        let start = egui::pos2(300., 300.);
        let middle = egui::pos2(350., 320.);
        let end = egui::pos2(380., 330.);
        navigation_frame(
            &mut preview,
            &context,
            vec![
                egui::Event::PointerMoved(start),
                pointer(start, egui::PointerButton::Primary, true),
            ],
        );
        navigation_frame(
            &mut preview,
            &context,
            vec![egui::Event::PointerMoved(middle)],
        );
        navigation_frame(
            &mut preview,
            &context,
            vec![pointer(end, egui::PointerButton::Primary, false)],
        );
        assert!((preview.camera.yaw - (Camera::default().yaw - 80. * 0.007)).abs() < 1e-6);
        assert!(
            (preview.camera.elevation - (Camera::default().elevation + 30. * 0.007)).abs() < 1e-6
        );
        assert!(preview.drag.is_none());
        navigation_frame(
            &mut preview,
            &context,
            vec![egui::Event::PointerMoved(egui::pos2(420., 400.))],
        );
        assert!((preview.camera.yaw - (Camera::default().yaw - 80. * 0.007)).abs() < 1e-6);
    }
}
