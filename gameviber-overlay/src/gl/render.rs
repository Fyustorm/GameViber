//! OpenGL renderer: draws the HUD meshes on the default framebuffer of the
//! current context right before the game swaps buffers, saving and restoring
//! every piece of GL state it touches.

use std::ffi::{c_void, CStr};
use std::num::NonZeroU32;

use epaint::{ImageData, Primitive, TextureId};
use gameviber_common::overlay::OverlayState;
use glow::HasContext;

use crate::hud::Hud;

const VERTEX_SIZE: i32 = 20;

const VERTEX: &str = r#"
uniform vec2 u_screen_size;
in vec2 a_pos;
in vec2 a_uv;
in vec4 a_color;
out vec2 v_uv;
out vec4 v_color;
void main() {
    gl_Position = vec4(2.0 * a_pos.x / u_screen_size.x - 1.0, 1.0 - 2.0 * a_pos.y / u_screen_size.y, 0.0, 1.0);
    v_uv = a_uv;
    v_color = a_color;
}
"#;

const FRAGMENT: &str = r#"
uniform sampler2D u_sampler;
in vec2 v_uv;
in vec4 v_color;
out vec4 f_color;
void main() {
    f_color = v_color * texture(u_sampler, v_uv);
}
"#;

/// Resources of one GL context (they cannot be shared with other contexts).
pub struct Renderer {
    gl: glow::Context,
    program: glow::Program,
    u_screen_size: Option<glow::UniformLocation>,
    u_sampler: Option<glow::UniformLocation>,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ibo: glow::Buffer,
    texture: glow::Texture,
    /// CPU copy of the font atlas; epaint sends partial updates.
    atlas: Option<epaint::ColorImage>,
    atlas_dirty: bool,
    hud: Hud,
}

impl Renderer {
    /// `load` resolves GL function names in the current context. Needs OpenGL 3.0 or OpenGL ES 3.0.
    pub unsafe fn new(load: impl FnMut(&CStr) -> *const c_void) -> Result<Self, String> {
        let gl = glow::Context::from_loader_function_cstr(load);
        let version = gl.version().clone();
        if version.major < 3 {
            return Err(format!("OpenGL {}.{} is too old (3.0 needed)", version.major, version.minor));
        }
        let header = if version.is_embedded {
            "#version 300 es\nprecision mediump float;\n"
        } else if (version.major, version.minor) >= (3, 3) {
            "#version 330\n"
        } else {
            "#version 130\n"
        };
        let program = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, source) in [(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, FRAGMENT)] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, &format!("{header}{source}"));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(format!("shader: {}", gl.get_shader_info_log(shader)));
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        for (i, name) in ["a_pos", "a_uv", "a_color"].into_iter().enumerate() {
            gl.bind_attrib_location(program, i as u32, name);
        }
        gl.link_program(program);
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if !gl.get_program_link_status(program) {
            return Err(format!("program: {}", gl.get_program_info_log(program)));
        }

        let saved = SavedState::save(&gl);
        let vao = gl.create_vertex_array()?;
        let vbo = gl.create_buffer()?;
        let ibo = gl.create_buffer()?;
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ibo));
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, VERTEX_SIZE, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, VERTEX_SIZE, 8);
        gl.enable_vertex_attrib_array(2);
        gl.vertex_attrib_pointer_f32(2, 4, glow::UNSIGNED_BYTE, true, VERTEX_SIZE, 16);
        let texture = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        saved.restore(&gl);

        Ok(Self {
            u_screen_size: gl.get_uniform_location(program, "u_screen_size"),
            u_sampler: gl.get_uniform_location(program, "u_sampler"),
            gl,
            program,
            vao,
            vbo,
            ibo,
            texture,
            atlas: None,
            atlas_dirty: false,
            hud: Hud::new(),
        })
    }

    pub fn version(&self) -> String {
        let v = self.gl.version();
        format!("OpenGL{} {}.{}", if v.is_embedded { " ES" } else { "" }, v.major, v.minor)
    }

    /// Draws the overlay on the default framebuffer of `width` x `height` pixels.
    pub unsafe fn draw(&mut self, state: &OverlayState, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        let frame = self.hud.build(state, width, height);
        if let Some(delta) = &frame.texture {
            let ImageData::Color(image) = &delta.image;
            match (delta.pos, &mut self.atlas) {
                (Some([x, y]), Some(atlas)) => {
                    let w = image.size[0];
                    for row in 0..image.size[1] {
                        let dst = (y + row) * atlas.size[0] + x;
                        atlas.pixels[dst..dst + w].copy_from_slice(&image.pixels[row * w..(row + 1) * w]);
                    }
                }
                _ => self.atlas = Some((**image).clone()),
            }
            self.atlas_dirty = true;
        }
        if frame.primitives.is_empty() {
            return;
        }
        let gl = &self.gl;
        let saved = SavedState::save(gl);
        self.prepare_state(width, height);

        let ppp = frame.pixels_per_point;
        gl.use_program(Some(self.program));
        gl.uniform_2_f32(self.u_screen_size.as_ref(), width as f32 / ppp, height as f32 / ppp);
        gl.uniform_1_i32(self.u_sampler.as_ref(), 0);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
        if saved.sampler.is_some() {
            gl.bind_sampler(0, None);
        }
        if self.atlas_dirty {
            if let Some(atlas) = &self.atlas {
                let bytes = std::slice::from_raw_parts(atlas.pixels.as_ptr() as *const u8, atlas.pixels.len() * 4);
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    atlas.size[0] as i32,
                    atlas.size[1] as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(bytes)),
                );
            }
            self.atlas_dirty = false;
        }
        gl.bind_vertex_array(Some(self.vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.ibo));
        for p in &frame.primitives {
            let Primitive::Mesh(mesh) = &p.primitive else { continue };
            if mesh.texture_id != TextureId::Managed(0) {
                continue;
            }
            let vertices = std::slice::from_raw_parts(mesh.vertices.as_ptr() as *const u8, mesh.vertices.len() * VERTEX_SIZE as usize);
            let indices = std::slice::from_raw_parts(mesh.indices.as_ptr() as *const u8, mesh.indices.len() * 4);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertices, glow::STREAM_DRAW);
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, indices, glow::STREAM_DRAW);
            // GL scissor boxes start at the bottom left.
            let clip = p.clip_rect;
            let x0 = (clip.min.x * ppp).round().clamp(0.0, width as f32) as i32;
            let x1 = (clip.max.x * ppp).round().clamp(0.0, width as f32) as i32;
            let y0 = (clip.min.y * ppp).round().clamp(0.0, height as f32) as i32;
            let y1 = (clip.max.y * ppp).round().clamp(0.0, height as f32) as i32;
            gl.scissor(x0, height as i32 - y1, (x1 - x0).max(0), (y1 - y0).max(0));
            gl.draw_elements(glow::TRIANGLES, mesh.indices.len() as i32, glow::UNSIGNED_INT, 0);
        }
        saved.restore(gl);
    }

    /// Fixed-function state for drawing premultiplied-alpha meshes on top.
    unsafe fn prepare_state(&self, width: u32, height: u32) {
        let gl = &self.gl;
        let desktop = !gl.version().is_embedded;
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
        gl.viewport(0, 0, width as i32, height as i32);
        gl.enable(glow::BLEND);
        gl.blend_equation_separate(glow::FUNC_ADD, glow::FUNC_ADD);
        gl.blend_func_separate(glow::ONE, glow::ONE_MINUS_SRC_ALPHA, glow::ONE_MINUS_DST_ALPHA, glow::ONE);
        for cap in [glow::CULL_FACE, glow::DEPTH_TEST, glow::STENCIL_TEST, glow::RASTERIZER_DISCARD] {
            gl.disable(cap);
        }
        gl.enable(glow::SCISSOR_TEST);
        gl.color_mask(true, true, true, true);
        if desktop {
            // epaint colors are gamma-space, like an 8-bit window.
            gl.disable(glow::FRAMEBUFFER_SRGB);
            gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
        }
        gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
        gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
    }
}

/// GL state the overlay changes, restored afterwards so the game never notices.
struct SavedState {
    program: i32,
    active_texture: i32,
    texture: i32,
    sampler: Option<i32>,
    array_buffer: i32,
    vertex_array: i32,
    unpack_buffer: i32,
    draw_framebuffer: i32,
    viewport: [i32; 4],
    scissor: [i32; 4],
    color_mask: [i32; 4],
    blend_src_rgb: i32,
    blend_dst_rgb: i32,
    blend_src_alpha: i32,
    blend_dst_alpha: i32,
    blend_eq_rgb: i32,
    blend_eq_alpha: i32,
    enabled: Vec<(u32, bool)>,
    polygon_mode: Option<i32>,
    unpack_alignment: i32,
    unpack_row_length: i32,
}

impl SavedState {
    unsafe fn save(gl: &glow::Context) -> Self {
        let v = gl.version();
        let desktop = !v.is_embedded;
        let has_samplers = v.is_embedded || (v.major, v.minor) >= (3, 3);
        let get = |p| gl.get_parameter_i32(p);
        let get4 = |p| {
            let mut out = [0; 4];
            gl.get_parameter_i32_slice(p, &mut out);
            out
        };
        let active_texture = get(glow::ACTIVE_TEXTURE);
        gl.active_texture(glow::TEXTURE0);
        let texture = get(glow::TEXTURE_BINDING_2D);
        let sampler = has_samplers.then(|| get(glow::SAMPLER_BINDING));
        gl.active_texture(active_texture as u32);
        let mut caps = vec![glow::BLEND, glow::CULL_FACE, glow::DEPTH_TEST, glow::STENCIL_TEST, glow::SCISSOR_TEST, glow::RASTERIZER_DISCARD];
        if desktop {
            caps.push(glow::FRAMEBUFFER_SRGB);
        }
        Self {
            program: get(glow::CURRENT_PROGRAM),
            active_texture,
            texture,
            sampler,
            array_buffer: get(glow::ARRAY_BUFFER_BINDING),
            vertex_array: get(glow::VERTEX_ARRAY_BINDING),
            unpack_buffer: get(glow::PIXEL_UNPACK_BUFFER_BINDING),
            draw_framebuffer: get(glow::DRAW_FRAMEBUFFER_BINDING),
            viewport: get4(glow::VIEWPORT),
            scissor: get4(glow::SCISSOR_BOX),
            color_mask: get4(glow::COLOR_WRITEMASK),
            blend_src_rgb: get(glow::BLEND_SRC_RGB),
            blend_dst_rgb: get(glow::BLEND_DST_RGB),
            blend_src_alpha: get(glow::BLEND_SRC_ALPHA),
            blend_dst_alpha: get(glow::BLEND_DST_ALPHA),
            blend_eq_rgb: get(glow::BLEND_EQUATION_RGB),
            blend_eq_alpha: get(glow::BLEND_EQUATION_ALPHA),
            enabled: caps.into_iter().map(|c| (c, gl.is_enabled(c))).collect(),
            polygon_mode: desktop.then(|| get4(glow::POLYGON_MODE)[0]),
            unpack_alignment: get(glow::UNPACK_ALIGNMENT),
            unpack_row_length: get(glow::UNPACK_ROW_LENGTH),
        }
    }

    unsafe fn restore(&self, gl: &glow::Context) {
        gl.use_program(name(self.program).map(glow::NativeProgram));
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, name(self.texture).map(glow::NativeTexture));
        if let Some(sampler) = self.sampler {
            gl.bind_sampler(0, name(sampler).map(glow::NativeSampler));
        }
        gl.active_texture(self.active_texture as u32);
        gl.bind_vertex_array(name(self.vertex_array).map(glow::NativeVertexArray));
        gl.bind_buffer(glow::ARRAY_BUFFER, name(self.array_buffer).map(glow::NativeBuffer));
        gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, name(self.unpack_buffer).map(glow::NativeBuffer));
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, name(self.draw_framebuffer).map(glow::NativeFramebuffer));
        let [x, y, w, h] = self.viewport;
        gl.viewport(x, y, w, h);
        let [x, y, w, h] = self.scissor;
        gl.scissor(x, y, w, h);
        let [r, g, b, a] = self.color_mask.map(|c| c != 0);
        gl.color_mask(r, g, b, a);
        gl.blend_equation_separate(self.blend_eq_rgb as u32, self.blend_eq_alpha as u32);
        gl.blend_func_separate(
            self.blend_src_rgb as u32,
            self.blend_dst_rgb as u32,
            self.blend_src_alpha as u32,
            self.blend_dst_alpha as u32,
        );
        for &(cap, on) in &self.enabled {
            if on {
                gl.enable(cap);
            } else {
                gl.disable(cap);
            }
        }
        if let Some(mode) = self.polygon_mode {
            gl.polygon_mode(glow::FRONT_AND_BACK, mode as u32);
        }
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, self.unpack_alignment);
        gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, self.unpack_row_length);
    }
}

/// A GL object name as glow wants it (0 is "none").
fn name(raw: i32) -> Option<NonZeroU32> {
    NonZeroU32::new(raw as u32)
}
