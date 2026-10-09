//! Windows: captures the game's sound through WASAPI: an application's own
//! sound (process loopback, Windows 10 2004 and later: the process and those
//! it started, whatever the device it plays on), or the default output's
//! loopback (everything the computer plays). The playing applications are the
//! audio sessions of the output devices.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::Context;
use windows::core::{implement, Interface, Ref, HRESULT, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    eConsole, eRender, ActivateAudioInterfaceAsync, AudioSessionStateExpired, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient,
    IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    DEVICE_STATE_ACTIVE, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX,
    WAVEFORMATEXTENSIBLE,
};
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, BLOB, CLSCTX_ALL, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::{
    CreateEventW, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::System::Variant::VT_BLOB;

use super::{Stream, Target};
use crate::audio::SAMPLE_RATE;

/// The capture's buffer, in 100 ns units.
const BUFFER: i64 = 200_000;
/// Mono f32 at `SAMPLE_RATE`, which process loopback converts to.
const CHANNELS: u16 = 1;

/// The playing applications: the audio sessions of every output device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Graph {
    /// Sorted by application name. `node` is the process id.
    pub streams: Vec<Stream>,
}

impl Graph {
    pub fn read() -> Self {
        com();
        let mut streams = sessions().unwrap_or_else(|e| {
            log::debug!("cannot list the audio sessions: {e}");
            Vec::new()
        });
        streams.sort_by(|a, b| (a.app.to_lowercase(), a.node).cmp(&(b.app.to_lowercase(), b.node)));
        streams.dedup_by_key(|s| s.node);
        Self { streams }
    }
}

/// COM for this thread (the audio thread, the capture thread), once.
fn com() {
    thread_local!(static COM: () = {
        // SAFETY: plain call; COM stays set up for the thread's life.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    });
    COM.with(|_| {});
}

fn sessions() -> windows::core::Result<Vec<Stream>> {
    let own = std::process::id();
    let mut streams = Vec::new();
    // SAFETY: COM calls on interfaces we hold; strings the sessions give are freed.
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        for d in 0..devices.GetCount()? {
            let Ok(manager) = devices.Item(d)?.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
            let list = manager.GetSessionEnumerator()?;
            for s in 0..list.GetCount()? {
                let Ok(control) = list.GetSession(s).and_then(|c| c.cast::<IAudioSessionControl2>()) else { continue };
                let pid = control.GetProcessId().unwrap_or(0);
                // The system's sounds, GameViber itself, sessions gone.
                if pid == 0 || pid == own || control.IsSystemSoundsSession().0 == 0 || control.GetState().is_ok_and(|st| st == AudioSessionStateExpired) {
                    continue;
                }
                let binary = process_name(pid).unwrap_or_default();
                let display = control.GetDisplayName().map(|name| take(name)).unwrap_or_default();
                // Display names are often resource references ("@%SystemRoot%\...").
                let app = if display.is_empty() || display.starts_with('@') { binary.trim_end_matches(".exe").to_owned() } else { display };
                if app.is_empty() {
                    continue;
                }
                streams.push(Stream { node: pid, app, binary, pid: Some(pid) });
            }
        }
    }
    Ok(streams)
}

/// A string COM gave us, freed.
unsafe fn take(text: PWSTR) -> String {
    let string = text.to_string().unwrap_or_default();
    CoTaskMemFree(Some(text.0 as _));
    string
}

/// The executable's file name of a process ("Game.exe").
fn process_name(pid: u32) -> Option<String> {
    // SAFETY: the handle is closed below; the buffer's size is passed.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut size);
        let _ = CloseHandle(process);
        result.ok()?;
        let path = String::from_utf16_lossy(&buffer[..size as usize]);
        path.rsplit('\\').next().map(str::to_owned)
    }
}

/// A capture running on a thread of its own, whose samples arrive on a channel.
pub struct Capture {
    pub target: Target,
    /// The process heard (an application's capture).
    pid: Option<u32>,
    stop: Arc<AtomicBool>,
    ended: Arc<AtomicBool>,
    pub samples: mpsc::Receiver<Vec<f32>>,
}

impl Capture {
    pub fn start(target: Target) -> anyhow::Result<Self> {
        let pid = match &target {
            Target::App { streams, .. } => Some(streams.iter().find_map(|s| s.pid).context("the application has no process")?),
            Target::Everything => None,
        };
        let (tx, samples) = mpsc::channel();
        let (stop, ended) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (started_tx, started_rx) = mpsc::channel();
        {
            let (stop, ended) = (stop.clone(), ended.clone());
            std::thread::Builder::new().name("audio-capture".into()).spawn(move || {
                com();
                match open(pid) {
                    Ok(stream) => {
                        let _ = started_tx.send(Ok(()));
                        if let Err(e) = stream.run(&stop, &tx) {
                            log::warn!("the sound capture stopped: {e}");
                        }
                    }
                    Err(e) => {
                        let _ = started_tx.send(Err(e));
                    }
                }
                ended.store(true, Ordering::Relaxed);
            })?;
        }
        started_rx.recv().context("the capture thread stopped")??;
        Ok(Self { target, pid, stop, ended, samples })
    }

    /// The same application keeps its capture while its process plays.
    pub fn update(&mut self, _graph: &Graph, target: &Target) -> bool {
        if !self.target.same_capture(target) {
            return false;
        }
        if let (Some(pid), Target::App { streams, .. }) = (self.pid, target) {
            if !streams.iter().any(|s| s.pid == Some(pid)) {
                return false;
            }
        }
        self.target = target.clone();
        true
    }

    pub fn ended(&mut self) -> bool {
        self.ended.load(Ordering::Relaxed)
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// An audio client capturing, and what its samples are.
struct Client {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    format: Format,
    /// Set when samples are ready (process loopback); polled otherwise.
    event: Option<HANDLE>,
}

// The client is only used by the capture thread, which made it (multithreaded COM).
unsafe impl Send for Client {}

fn open(pid: Option<u32>) -> anyhow::Result<Client> {
    // SAFETY: COM calls on interfaces we hold; the formats outlive Initialize.
    unsafe {
        let (client, format, event) = match pid {
            Some(pid) => {
                let client = process_client(pid).context("cannot capture the application's sound (Windows 10 2004 or later needed)")?;
                let block = CHANNELS * 4;
                let wave = WAVEFORMATEX {
                    wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
                    nChannels: CHANNELS,
                    nSamplesPerSec: SAMPLE_RATE,
                    nAvgBytesPerSec: SAMPLE_RATE * u32::from(block),
                    nBlockAlign: block,
                    wBitsPerSample: 32,
                    cbSize: 0,
                };
                let event = CreateEventW(None, false, false, None)?;
                client
                    .Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK, BUFFER, 0, &wave, None)
                    .context("cannot start the application's capture")?;
                client.SetEventHandle(event)?;
                (client, Format::of(&wave)?, Some(event))
            }
            None => {
                let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).context("no sound output")?;
                let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
                let mix = client.GetMixFormat()?;
                let format = Format::of(&*mix);
                let result = client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, BUFFER, 0, mix, None);
                CoTaskMemFree(Some(mix as _));
                result.context("cannot capture the sound output")?;
                (client, format?, None)
            }
        };
        let capture: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        Ok(Client { client, capture, format, event })
    }
}

/// Process loopback's audio client: activated asynchronously, waited for.
unsafe fn process_client(pid: u32) -> anyhow::Result<IAudioClient> {
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    let blob = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB { cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32, pBlobData: &mut params as *mut _ as *mut u8 },
                },
            }),
        },
    };
    let done = Arc::new((Mutex::new(false), Condvar::new()));
    let handler: IActivateAudioInterfaceCompletionHandler = Activated(done.clone()).into();
    let operation = ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, Some(&blob), &handler)?;
    let (lock, ready) = &*done;
    let (finished, _) = ready.wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(5), |done| !*done).unwrap();
    anyhow::ensure!(*finished, "Windows did not answer");
    let mut result = HRESULT(0);
    let mut client = None;
    operation.GetActivateResult(&mut result, &mut client)?;
    result.ok()?;
    Ok(client.context("no audio client")?.cast()?)
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Activated(Arc<(Mutex<bool>, Condvar)>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
    fn ActivateCompleted(&self, _operation: Ref<IActivateAudioInterfaceAsyncOperation>) -> windows::core::Result<()> {
        let (lock, ready) = &*self.0;
        *lock.lock().unwrap() = true;
        ready.notify_all();
        Ok(())
    }
}

impl Client {
    fn run(&self, stop: &AtomicBool, tx: &mpsc::Sender<Vec<f32>>) -> anyhow::Result<()> {
        let mut converter = Converter::new(self.format);
        while !stop.load(Ordering::Relaxed) {
            // SAFETY: COM calls on interfaces we hold; the buffer is read before it is released.
            unsafe {
                match self.event {
                    Some(event) => {
                        // A timeout is fine: nothing playing.
                        let _ = WaitForSingleObject(event, 100) == WAIT_OBJECT_0;
                    }
                    None => std::thread::sleep(Duration::from_millis(10)),
                }
                let mut samples = Vec::new();
                while self.capture.GetNextPacketSize()? > 0 {
                    let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
                    self.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                    let bytes = frames as usize * self.format.frame_bytes();
                    if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                        converter.silence(frames as usize, &mut samples);
                    } else {
                        converter.push(std::slice::from_raw_parts(data, bytes), &mut samples);
                    }
                    self.capture.ReleaseBuffer(frames)?;
                }
                if !samples.is_empty() && tx.send(samples).is_err() {
                    break;
                }
            }
        }
        Ok(())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: plain calls on what we own.
        unsafe {
            let _ = self.client.Stop();
            if let Some(event) = self.event {
                let _ = CloseHandle(event);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Sample {
    F32,
    I16,
    I24,
    I32,
}

/// What a device's samples are.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Format {
    sample: Sample,
    channels: usize,
    rate: u32,
}

impl Format {
    /// From a `WAVEFORMATEX` (or the `WAVEFORMATEXTENSIBLE` it starts).
    ///
    /// # Safety
    /// `wave` is a whole `WAVEFORMATEXTENSIBLE` when its tag says so.
    unsafe fn of(wave: &WAVEFORMATEX) -> anyhow::Result<Self> {
        let tag = u32::from(wave.wFormatTag);
        let float = match tag {
            WAVE_FORMAT_IEEE_FLOAT => true,
            1 => false,
            WAVE_FORMAT_EXTENSIBLE => {
                let sub = (*(wave as *const WAVEFORMATEX as *const WAVEFORMATEXTENSIBLE)).SubFormat;
                match sub {
                    s if s == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT => true,
                    s if s == KSDATAFORMAT_SUBTYPE_PCM => false,
                    _ => anyhow::bail!("unknown sample format"),
                }
            }
            _ => anyhow::bail!("unknown sample format {tag}"),
        };
        let bits = if wave.nChannels > 0 { wave.nBlockAlign / wave.nChannels * 8 } else { 0 };
        let sample = match (float, bits) {
            (true, 32) => Sample::F32,
            (false, 16) => Sample::I16,
            (false, 24) => Sample::I24,
            (false, 32) => Sample::I32,
            _ => anyhow::bail!("unknown sample size ({bits} bits)"),
        };
        Ok(Self { sample, channels: usize::from(wave.nChannels.max(1)), rate: wave.nSamplesPerSec })
    }

    fn sample_bytes(&self) -> usize {
        match self.sample {
            Sample::F32 | Sample::I32 => 4,
            Sample::I16 => 2,
            Sample::I24 => 3,
        }
    }

    fn frame_bytes(&self) -> usize {
        self.sample_bytes() * self.channels
    }
}

/// A device's frames to mono f32 at `SAMPLE_RATE` (channels averaged, linear resampling).
struct Converter {
    format: Format,
    /// Where the next output sample falls, in input frames from `last`.
    position: f64,
    /// The last input frame, mono.
    last: f32,
}

impl Converter {
    fn new(format: Format) -> Self {
        Self { format, position: 0.0, last: 0.0 }
    }

    fn mono(&self, frame: &[u8]) -> f32 {
        let size = self.format.sample_bytes();
        let sum: f32 = frame
            .chunks_exact(size)
            .map(|b| match self.format.sample {
                Sample::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                Sample::I16 => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0,
                Sample::I24 => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
                Sample::I32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
            })
            .sum();
        sum / self.format.channels as f32
    }

    fn push(&mut self, bytes: &[u8], out: &mut Vec<f32>) {
        let frames: Vec<f32> = bytes.chunks_exact(self.format.frame_bytes()).map(|f| self.mono(f)).collect();
        self.resample(&frames, out);
    }

    fn silence(&mut self, frames: usize, out: &mut Vec<f32>) {
        self.resample(&vec![0.0; frames], out);
    }

    fn resample(&mut self, frames: &[f32], out: &mut Vec<f32>) {
        if self.format.rate == SAMPLE_RATE {
            out.extend_from_slice(frames);
            return;
        }
        let step = f64::from(self.format.rate) / f64::from(SAMPLE_RATE);
        // Between `last` (0) and the new frames (1 to len): one input frame of delay.
        let last = self.last;
        let frame = |k: usize| if k == 0 { last } else { frames[k - 1] };
        while self.position < frames.len() as f64 {
            let i = self.position.floor() as usize;
            let t = (self.position - i as f64) as f32;
            let (a, b) = (frame(i), frame(i + 1));
            out.push(a + (b - a) * t);
            self.position += step;
        }
        self.position -= frames.len() as f64;
        if let Some(&last) = frames.last() {
            self.last = last;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_become_mono_samples_at_our_rate() {
        let stereo = Format { sample: Sample::I16, channels: 2, rate: SAMPLE_RATE };
        let mut c = Converter::new(stereo);
        let mut out = Vec::new();
        let frame = |l: i16, r: i16| [l.to_le_bytes(), r.to_le_bytes()].concat();
        c.push(&[frame(16384, 0), frame(-32768, -32768)].concat(), &mut out);
        assert_eq!(out, [0.25, -1.0], "channels averaged");

        let half = Format { sample: Sample::F32, channels: 1, rate: SAMPLE_RATE / 2 };
        let mut c = Converter::new(half);
        let mut out = Vec::new();
        let bytes: Vec<u8> = [1.0f32, 1.0, 1.0, 1.0].iter().flat_map(|s| s.to_le_bytes()).collect();
        c.push(&bytes, &mut out);
        c.push(&bytes, &mut out);
        assert_eq!(out.len(), 16, "twice as many samples");
        assert!(out[2..].iter().all(|s| *s == 1.0), "once past the first frame: {out:?}");

        let mut c = Converter::new(Format { sample: Sample::I24, channels: 1, rate: 44_100 });
        let mut out = Vec::new();
        c.silence(44_100, &mut out);
        assert!((out.len() as i64 - i64::from(SAMPLE_RATE)).abs() <= 1, "{}", out.len());
        let mut c = Converter::new(Format { sample: Sample::I24, channels: 1, rate: SAMPLE_RATE });
        let mut out = Vec::new();
        c.push(&[0x00, 0x00, 0xc0], &mut out);
        assert_eq!(out, [-0.5], "24 bits");
    }

    #[test]
    fn formats_are_read_from_the_device() {
        let wave = |tag: u32, channels: u16, bits: u16| WAVEFORMATEX {
            wFormatTag: tag as u16,
            nChannels: channels,
            nSamplesPerSec: 44_100,
            nBlockAlign: channels * bits / 8,
            wBitsPerSample: bits,
            ..Default::default()
        };
        let read = |w: WAVEFORMATEX| unsafe { Format::of(&w) }.ok();
        assert_eq!(read(wave(WAVE_FORMAT_IEEE_FLOAT, 2, 32)), Some(Format { sample: Sample::F32, channels: 2, rate: 44_100 }));
        assert_eq!(read(wave(1, 6, 24)).map(|f| f.sample), Some(Sample::I24));
        assert_eq!(read(wave(2, 2, 16)), None, "ADPCM");
    }
}
