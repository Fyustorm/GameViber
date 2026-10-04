//! Copies of the game's image in OpenGL: the back buffer is blitted into a
//! texture, halved through its mip levels (`glGenerateMipmap` averages), and
//! the last level is read into a pixel buffer without waiting; the pixels are
//! picked up once a fence says the GPU is done.

use gameviber_common::overlay::frames;
use glow::HasContext;

use crate::linux::client::Client;

/// At most this many halvings after the first blit.
const MAX_HALVINGS: u32 = 6;

pub struct Capture {
    texture: glow::Texture,
    /// Level 0 of the texture, blitted to; and its last level, read from.
    top_fbo: glow::Framebuffer,
    last_fbo: glow::Framebuffer,
    pbo: glow::Buffer,
    top: (u32, u32),
    halvings: u32,
    /// The requested width and the image size it was made for.
    made_for: (u32, (u32, u32)),
    /// A copy was read into the pixel buffer from an image of this size.
    pending: Option<((u32, u32), glow::Fence)>,
}

impl Capture {
    fn size(&self) -> (u32, u32) {
        (self.top.0 >> self.halvings, self.top.1 >> self.halvings)
    }

    unsafe fn new(gl: &glow::Context, width: u32, source: (u32, u32)) -> Result<Self, String> {
        let (w, h) = frames::copy_size(width, source);
        let mut halvings = 0;
        while halvings < MAX_HALVINGS && (w << (halvings + 1)) <= source.0 && (h << (halvings + 1)) <= source.1 {
            halvings += 1;
        }
        let top = (w << halvings, h << halvings);
        let texture = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        for level in 0..=halvings {
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                level as i32,
                glow::RGBA8 as i32,
                (top.0 >> level) as i32,
                (top.1 >> level) as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
        }
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAX_LEVEL, halvings as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR as i32);
        let fbo = |level: u32| -> Result<glow::Framebuffer, String> {
            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(glow::DRAW_FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(texture), level as i32);
            Ok(fbo)
        };
        let top_fbo = fbo(0)?;
        let last_fbo = fbo(halvings)?;
        let pbo = gl.create_buffer()?;
        gl.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(pbo));
        gl.buffer_data_size(glow::PIXEL_PACK_BUFFER, (w * h * 4) as i32, glow::STREAM_READ);
        Ok(Self { texture, top_fbo, last_fbo, pbo, top, halvings, made_for: (width, source), pending: None })
    }

    pub unsafe fn destroy(self, gl: &glow::Context) {
        if let Some((_, fence)) = self.pending {
            gl.delete_sync(fence);
        }
        gl.delete_framebuffer(self.top_fbo);
        gl.delete_framebuffer(self.last_fbo);
        gl.delete_buffer(self.pbo);
        gl.delete_texture(self.texture);
    }
}

/// Picks up a finished copy, and starts a new one when `client` wants it.
/// The caller saves and restores the GL state (framebuffers, pack buffer, texture).
pub unsafe fn update(gl: &glow::Context, slot: &mut Option<Capture>, client: &mut Client, width: u32, height: u32) {
    if let Some(capture) = slot.as_mut() {
        publish(gl, capture, client);
    }
    let Some(request) = client.capture_request() else { return };
    if slot.as_ref().is_some_and(|c| c.pending.is_some()) || !client.capture_due() {
        return;
    }
    if slot.as_ref().is_some_and(|c| c.made_for != (request.width, (width, height))) {
        if let Some(old) = slot.take() {
            old.destroy(gl);
        }
    }
    if slot.is_none() {
        match Capture::new(gl, request.width, (width, height)) {
            Ok(c) => *slot = Some(c),
            Err(e) => {
                crate::log(&format!("cannot copy the game's image: {e}"));
                return;
            }
        }
    }
    let capture = slot.as_mut().unwrap();
    gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
    gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(capture.top_fbo));
    gl.blit_framebuffer(
        0,
        0,
        width as i32,
        height as i32,
        0,
        0,
        capture.top.0 as i32,
        capture.top.1 as i32,
        glow::COLOR_BUFFER_BIT,
        glow::LINEAR,
    );
    gl.bind_texture(glow::TEXTURE_2D, Some(capture.texture));
    gl.generate_mipmap(glow::TEXTURE_2D);
    let (w, h) = capture.size();
    gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(capture.last_fbo));
    gl.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(capture.pbo));
    gl.pixel_store_i32(glow::PACK_ALIGNMENT, 4);
    gl.pixel_store_i32(glow::PACK_ROW_LENGTH, 0);
    gl.read_pixels(0, 0, w as i32, h as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::BufferOffset(0));
    if let Ok(fence) = gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0) {
        capture.pending = Some(((width, height), fence));
    }
}

/// Hands a finished copy over to GameViber, never waiting for the GPU.
unsafe fn publish(gl: &glow::Context, capture: &mut Capture, client: &mut Client) {
    let Some((source, fence)) = capture.pending else { return };
    let status = gl.client_wait_sync(fence, 0, 0);
    if status != glow::ALREADY_SIGNALED && status != glow::CONDITION_SATISFIED {
        return;
    }
    gl.delete_sync(fence);
    capture.pending = None;
    let (w, h) = capture.size();
    let row = w as usize * 4;
    gl.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(capture.pbo));
    let ptr = gl.map_buffer_range(glow::PIXEL_PACK_BUFFER, 0, (row * h as usize) as i32, glow::MAP_READ_BIT);
    if ptr.is_null() {
        return;
    }
    // GL rows go bottom to top.
    let read = std::slice::from_raw_parts(ptr, row * h as usize);
    let flipped: Vec<u8> = read.chunks_exact(row).rev().flatten().copied().collect();
    gl.unmap_buffer(glow::PIXEL_PACK_BUFFER);
    if let Some(shared) = client.frames() {
        shared.publish(w, h, source, &flipped, row, false);
    }
}
