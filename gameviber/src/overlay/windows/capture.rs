//! The game window's image, through Windows.Graphics.Capture (Windows 10 1903
//! and later): on a thread of its own, each frame is cropped to the window's
//! client area, made smaller on the GPU (mipmaps), read back and scaled to the
//! size GameViber asked for, then written into frame memory like the in-game
//! overlay's copies (`gameviber_common::overlay::frames`). Nothing is injected
//! into the game; a game in exclusive fullscreen may give black frames.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::Context;
use gameviber_common::overlay::frames;
use windows::core::{factory, Interface};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, HWND, POINT, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET,
    D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_RESOURCE_MISC_GENERATE_MIPS, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, IsWindow};

use super::Frames;

/// A window being captured.
pub struct WindowCapture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WindowCapture {
    /// Captures `window` (an `HWND`) into `frames`, `width` pixels wide, `fps` times a second.
    pub fn start(window: isize, frames: Arc<Frames>, width: u32, fps: f32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("window-capture".into())
                .spawn(move || {
                    // SAFETY: WinRT for this thread's life.
                    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
                    if let Err(e) = run(HWND(window as _), &frames, width, fps, &stop) {
                        log::warn!("cannot read the game's image: {e:#}");
                    }
                })
                .ok()
        };
        Self { stop, thread }
    }

    /// The capture stopped by itself (window closed, capture refused).
    pub fn ended(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for WindowCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The GPU side: a device, and the textures each frame goes through.
struct Gpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    /// The client area, with its mipmaps.
    mips: Option<(ID3D11Texture2D, ID3D11ShaderResourceView, u32, u32)>,
    /// The mipmap read back, and its size.
    staging: Option<(ID3D11Texture2D, u32, u32)>,
}

fn run(window: HWND, frames: &Frames, width: u32, fps: f32, stop: &AtomicBool) -> anyhow::Result<()> {
    anyhow::ensure!(GraphicsCaptureSession::IsSupported().unwrap_or(false), "Windows cannot capture windows here (Windows 10 1903 or later needed)");
    // SAFETY: Direct3D and WinRT calls on objects this thread made.
    unsafe {
        let mut device = None;
        let mut context = None;
        D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context))
            .context("no Direct3D 11 device")?;
        let (device, context): (ID3D11Device, ID3D11DeviceContext) = (device.context("no device")?, context.context("no context")?);
        let dxgi: IDXGIDevice = device.cast()?;
        let winrt_device: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
        let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = interop.CreateForWindow(window).context("the window cannot be captured")?;
        let mut size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&winrt_device, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size)?;
        let session = pool.CreateCaptureSession(&item)?;
        // Windows 11: no yellow border around the game, no cursor in the image.
        let _ = session.SetIsBorderRequired(false);
        let _ = session.SetIsCursorCaptureEnabled(false);
        session.StartCapture()?;
        log::info!("reading the game's window ({}x{})", size.Width, size.Height);
        let mut gpu = Gpu { device, context, mips: None, staging: None };
        let period = Duration::from_secs_f32(1.0 / fps.clamp(0.5, 60.0));
        let mut next = Instant::now();
        let result = loop {
            if stop.load(Ordering::Relaxed) {
                break Ok(());
            }
            if !IsWindow(Some(window)).as_bool() {
                break Ok(());
            }
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
            next = Instant::now() + period;
            // The newest frame; older ones are dropped.
            let mut latest = None;
            while let Ok(frame) = pool.TryGetNextFrame() {
                latest = Some(frame);
            }
            let Some(frame) = latest else { continue };
            let content = frame.ContentSize()?;
            if content.Width != size.Width || content.Height != size.Height {
                size = content;
                pool.Recreate(&winrt_device, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size)?;
                continue;
            }
            let texture: ID3D11Texture2D = frame.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()?;
            if let Err(e) = gpu.copy(window, &texture, size, frames, width) {
                break Err(e);
            }
            let _ = frame.Close();
        };
        let _ = session.Close();
        let _ = pool.Close();
        result
    }
}

impl Gpu {
    /// One frame of the window into frame memory.
    unsafe fn copy(&mut self, window: HWND, texture: &ID3D11Texture2D, size: SizeInt32, frames: &Frames, width: u32) -> anyhow::Result<()> {
        let (full_w, full_h) = (size.Width.max(1) as u32, size.Height.max(1) as u32);
        let client = client_area(window, full_w, full_h);
        let (cw, ch) = (client.right - client.left, client.bottom - client.top);
        if cw < 16 || ch < 16 {
            return Ok(());
        }
        let (tw, th) = frames::copy_size(width, (cw, ch));
        // The client area, then its mipmaps down to the first one still as large as the copy.
        if self.mips.as_ref().is_none_or(|(_, _, w, h)| (*w, *h) != (cw, ch)) {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: cw,
                Height: ch,
                MipLevels: 0,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
            };
            let mut mips = None;
            self.device.CreateTexture2D(&desc, None, Some(&mut mips))?;
            let mips = mips.context("no texture")?;
            let mut view = None;
            self.device.CreateShaderResourceView(&mips, None, Some(&mut view))?;
            self.mips = Some((mips, view.context("no view")?, cw, ch));
        }
        let (mips, view, _, _) = self.mips.as_ref().expect("made above");
        let source = D3D11_BOX { left: client.left, top: client.top, front: 0, right: client.right, bottom: client.bottom, back: 1 };
        self.context.CopySubresourceRegion(mips, 0, 0, 0, 0, texture, 0, Some(&source));
        let level = mip_level((cw, ch), (tw, th));
        if level > 0 {
            self.context.GenerateMips(view);
        }
        let (lw, lh) = ((cw >> level).max(1), (ch >> level).max(1));
        if self.staging.as_ref().is_none_or(|(_, w, h)| (*w, *h) != (lw, lh)) {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: lw,
                Height: lh,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut staging = None;
            self.device.CreateTexture2D(&desc, None, Some(&mut staging))?;
            self.staging = Some((staging.context("no texture")?, lw, lh));
        }
        let (staging, _, _) = self.staging.as_ref().expect("made above");
        self.context.CopySubresourceRegion(staging, 0, 0, 0, 0, mips, level, None);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // Waits for the copy: this thread only.
        self.context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        let pitch = mapped.RowPitch as usize;
        let source = std::slice::from_raw_parts(mapped.pData as *const u8, pitch * lh as usize);
        frames::write(frames.base(), tw, th, (cw, ch), |pixels| downscale(source, pitch, (lw, lh), pixels, (tw, th)));
        self.context.Unmap(staging, 0);
        Ok(())
    }
}

/// The window's client area inside the captured image (which holds its frame too).
unsafe fn client_area(window: HWND, full_w: u32, full_h: u32) -> Area {
    let whole = RECT { left: 0, top: 0, right: full_w as i32, bottom: full_h as i32 };
    let mut bounds = RECT::default();
    let mut client = RECT::default();
    let mut origin = POINT::default();
    if DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS, &mut bounds as *mut RECT as _, std::mem::size_of::<RECT>() as u32).is_err()
        || GetClientRect(window, &mut client).is_err()
        || !ClientToScreen(window, &mut origin).as_bool()
    {
        return as_box(whole);
    }
    let left = (origin.x - bounds.left).clamp(0, full_w as i32);
    let top = (origin.y - bounds.top).clamp(0, full_h as i32);
    let right = (left + client.right - client.left).clamp(left, full_w as i32);
    let bottom = (top + client.bottom - client.top).clamp(top, full_h as i32);
    as_box(RECT { left, top, right, bottom })
}

/// A rectangle with unsigned corners, as the copy takes it.
struct Area {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
}

fn as_box(r: RECT) -> Area {
    Area { left: r.left.max(0) as u32, top: r.top.max(0) as u32, right: r.right.max(0) as u32, bottom: r.bottom.max(0) as u32 }
}

/// The smallest mipmap still at least the copy's size.
fn mip_level(source: (u32, u32), copy: (u32, u32)) -> u32 {
    let mut level = 0;
    while (source.0 >> (level + 1)) >= copy.0 && (source.1 >> (level + 1)) >= copy.1 && level < 12 {
        level += 1;
    }
    level
}

/// BGRA rows `pitch` bytes apart, `from` pixels, averaged into RGBA `to` pixels.
fn downscale(source: &[u8], pitch: usize, from: (u32, u32), out: &mut [u8], to: (u32, u32)) {
    let (fw, fh) = (from.0 as usize, from.1 as usize);
    let (tw, th) = (to.0 as usize, to.1 as usize);
    for y in 0..th {
        let (y0, y1) = (y * fh / th, ((y + 1) * fh / th).max(y * fh / th + 1).min(fh));
        for x in 0..tw {
            let (x0, x1) = (x * fw / tw, ((x + 1) * fw / tw).max(x * fw / tw + 1).min(fw));
            let mut sum = [0u32; 3];
            for sy in y0..y1 {
                let row = &source[sy * pitch..];
                for sx in x0..x1 {
                    let p = &row[sx * 4..sx * 4 + 4];
                    sum[0] += u32::from(p[2]);
                    sum[1] += u32::from(p[1]);
                    sum[2] += u32::from(p[0]);
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let at = (y * tw + x) * 4;
            out[at..at + 4].copy_from_slice(&[(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, 255]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_averaged_into_rgba() {
        // 4x2 BGRA, rows padded to 20 bytes: left half blue, right half red.
        let mut source = vec![0u8; 40];
        for y in 0..2 {
            for x in 0..4 {
                let p = &mut source[y * 20 + x * 4..y * 20 + x * 4 + 4];
                p.copy_from_slice(if x < 2 { &[255, 0, 0, 0] } else { &[0, 0, 255, 0] });
            }
        }
        let mut out = vec![0u8; 2 * 1 * 4];
        downscale(&source, 20, (4, 2), &mut out, (2, 1));
        assert_eq!(out, [0, 0, 255, 255, 255, 0, 0, 255], "blue then red, opaque");
    }

    #[test]
    fn the_mipmap_read_is_as_small_as_the_copy_allows() {
        assert_eq!(mip_level((1920, 1080), (480, 270)), 2);
        assert_eq!(mip_level((1920, 1080), (960, 540)), 1);
        assert_eq!(mip_level((500, 300), (480, 288)), 0);
    }
}
