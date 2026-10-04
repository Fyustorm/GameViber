//! CLAP audio scene model (LAION `larger_clap_music_and_speech`, quantized
//! ONNX export): 10 s of sound and short texts are turned into vectors
//! whose similarity tells how well a text describes the sound. Modes describe
//! their scenes in words ("intense battle music"); this module measures how
//! much the game's sound looks like each.
//!
//! The model is downloaded on demand (about 200 MB) and runs on the CPU.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context};
use ort::session::Session;
use ort::value::Tensor;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

use super::SAMPLE_RATE;
use crate::config;

/// Length of the sound the model looks at: 10 s.
pub const CLIP_SAMPLES: usize = 480_000;
/// `exp(logit_scale_a)` of the model: sharpness of the scene probabilities.
pub const LOGIT_SCALE: f64 = 27.4399;
const N_FFT: usize = 1024;
const HOP: usize = 480;
const N_MELS: usize = 64;
const FRAMES: usize = CLIP_SAMPLES / HOP + 1;
const MEL_MIN_HZ: f64 = 50.0;
const MEL_MAX_HZ: f64 = 14_000.0;
/// Threads each model may use: the game needs the CPU more.
const THREADS: usize = 2;

const REPO: &str = "https://huggingface.co/Xenova/larger_clap_music_and_speech/resolve";
/// Pinned revision of the repository, so that the files never change under us.
const REVISION: &str = "e9fd5ac1dbf3280936a7fc3ec8a020453ff184db";
const AUDIO_MODEL: &str = "audio_model_quantized.onnx";
const TEXT_MODEL: &str = "text_model_quantized.onnx";
const TOKENIZER: &str = "tokenizer.json";
/// Files to download: repository path, local name, size in bytes.
const FILES: [(&str, &str, u64); 3] = [
    ("onnx/audio_model_quantized.onnx", AUDIO_MODEL, 78_155_433),
    ("onnx/text_model_quantized.onnx", TEXT_MODEL, 126_603_262),
    ("tokenizer.json", TOKENIZER, 2_108_774),
];
pub const DOWNLOAD_SIZE: u64 = FILES[0].2 + FILES[1].2 + FILES[2].2;

/// A unit vector describing a sound or a text.
pub type Embedding = Arc<[f32]>;

#[derive(Debug, Clone, Default, PartialEq)]
pub enum ModelState {
    #[default]
    Missing,
    Downloading { done: u64, total: u64 },
    Ready,
    Failed(String),
}

static STATE: Mutex<Option<ModelState>> = Mutex::new(None);

pub fn model_dir() -> PathBuf {
    config::data_dir().join("models").join("clap-music-speech")
}

fn files_present() -> bool {
    let dir = model_dir();
    FILES.iter().all(|(_, name, size)| fs::metadata(dir.join(name)).is_ok_and(|m| m.len() == *size))
}

pub fn model_state() -> ModelState {
    let mut state = STATE.lock().unwrap();
    state.get_or_insert_with(|| if files_present() { ModelState::Ready } else { ModelState::Missing }).clone()
}

pub fn model_ready() -> bool {
    model_state() == ModelState::Ready
}

fn set_state(state: ModelState) {
    *STATE.lock().unwrap() = Some(state);
}

/// Downloads the model in the background; `model_state()` follows the progress.
pub fn start_download() {
    if matches!(model_state(), ModelState::Ready | ModelState::Downloading { .. }) {
        return;
    }
    set_state(ModelState::Downloading { done: 0, total: DOWNLOAD_SIZE });
    std::thread::Builder::new()
        .name("model-download".into())
        .spawn(|| match download() {
            Ok(()) => {
                log::info!("audio scene model downloaded to {}", model_dir().display());
                set_state(ModelState::Ready);
            }
            Err(e) => {
                log::error!("audio scene model download failed: {e:#}");
                set_state(ModelState::Failed(format!("{e:#}")));
            }
        })
        .expect("spawn the download thread");
}

fn download() -> anyhow::Result<()> {
    let dir = model_dir();
    config::create_dir(&dir)?;
    let mut done = 0;
    for (path, name, size) in FILES {
        let target = dir.join(name);
        if fs::metadata(&target).is_ok_and(|m| m.len() == size) {
            done += size;
            continue;
        }
        let url = format!("{REPO}/{REVISION}/{path}");
        let response = ureq::get(&url).call().with_context(|| format!("downloading {url}"))?;
        let mut body = response.into_body().into_reader();
        let partial = dir.join(format!("{name}.part"));
        let mut file = fs::File::create(&partial)?;
        let mut buffer = vec![0; 1 << 16];
        let mut received = 0;
        loop {
            let n = body.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n])?;
            received += n as u64;
            set_state(ModelState::Downloading { done: done + received, total: DOWNLOAD_SIZE });
        }
        file.flush()?;
        anyhow::ensure!(received == size, "{name}: got {received} bytes instead of {size}");
        fs::rename(&partial, &target)?;
        done += size;
    }
    Ok(())
}

fn session(name: &str) -> anyhow::Result<Session> {
    let path = model_dir().join(name);
    let builder = Session::builder()?;
    let builder = builder.with_intra_threads(THREADS).map_err(|e| anyhow!("{e}"))?;
    builder
        .with_inter_threads(1)
        .map_err(|e| anyhow!("{e}"))?
        .commit_from_file(&path)
        .map_err(|e| anyhow!("loading {}: {e}", path.display()))
}

fn normalize(v: &[f32]) -> Embedding {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    v.iter().map(|x| x / norm).collect()
}

/// Turns 10 s clips of sound into embeddings.
pub struct AudioEncoder {
    session: Session,
    mel: MelSpectrogram,
}

impl AudioEncoder {
    pub fn load() -> anyhow::Result<Self> {
        Ok(Self { session: session(AUDIO_MODEL)?, mel: MelSpectrogram::new() })
    }

    /// `clip`: `CLIP_SAMPLES` mono samples at `SAMPLE_RATE`.
    pub fn embed(&mut self, clip: &[f32]) -> anyhow::Result<Embedding> {
        let features = self.mel.compute(clip);
        let input = Tensor::from_array(([1usize, 1, FRAMES, N_MELS], features))?;
        let outputs = self.session.run(ort::inputs!["input_features" => input])?;
        let (_, embedding) = outputs["audio_embeds"].try_extract_tensor::<f32>()?;
        Ok(normalize(embedding))
    }
}

/// Text embeddings, computed once per description. The text model is only
/// loaded while new descriptions are encoded.
static TEXTS: Mutex<Option<HashMap<String, Embedding>>> = Mutex::new(None);

pub fn text_embeddings(descriptions: &[String]) -> anyhow::Result<Vec<Embedding>> {
    let mut cache = TEXTS.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);
    let missing: Vec<&String> = descriptions.iter().filter(|d| !cache.contains_key(*d)).collect();
    if !missing.is_empty() {
        anyhow::ensure!(model_ready(), "the audio scene model is not downloaded");
        let tokenizer = tokenizers::Tokenizer::from_file(model_dir().join(TOKENIZER)).map_err(|e| anyhow!("tokenizer: {e}"))?;
        let mut session = session(TEXT_MODEL)?;
        for description in missing {
            let encoding = tokenizer.encode(description.as_str(), true).map_err(|e| anyhow!("tokenizer: {e}"))?;
            let ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
            let input = Tensor::from_array(([1usize, ids.len()], ids))?;
            let outputs = session.run(ort::inputs!["input_ids" => input])?;
            let (_, embedding) = outputs["text_embeds"].try_extract_tensor::<f32>()?;
            cache.insert(description.clone(), normalize(embedding));
        }
    }
    Ok(descriptions.iter().map(|d| cache[d].clone()).collect())
}

/// How well each text describes the sound, summing to 1.
pub fn probabilities(sound: &[f32], texts: &[Embedding]) -> Vec<f64> {
    let logits: Vec<f64> =
        texts.iter().map(|t| LOGIT_SCALE * t.iter().zip(sound).map(|(a, b)| (a * b) as f64).sum::<f64>()).collect();
    let max = logits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let exp: Vec<f64> = logits.iter().map(|l| (l - max).exp()).collect();
    let sum: f64 = exp.iter().sum();
    exp.into_iter().map(|e| e / sum).collect()
}

/// The model's input: log-mel spectrogram of a 10 s clip, as computed by
/// `transformers`' `ClapFeatureExtractor` (centered frames with reflect
/// padding, periodic Hann window, power spectrum, Slaney mel filters, dB).
struct MelSpectrogram {
    fft: Arc<dyn rustfft::Fft<f64>>,
    window: Vec<f64>,
    /// `N_FFT / 2 + 1` rows of `N_MELS` weights.
    filters: Vec<[f64; N_MELS]>,
}

impl MelSpectrogram {
    fn new() -> Self {
        Self {
            fft: FftPlanner::new().plan_fft_forward(N_FFT),
            window: (0..N_FFT).map(|n| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / N_FFT as f64).cos()).collect(),
            filters: mel_filters(),
        }
    }

    /// `FRAMES` x `N_MELS` values, frame by frame.
    fn compute(&self, clip: &[f32]) -> Vec<f32> {
        let n = clip.len();
        let pad = N_FFT / 2;
        // numpy's "reflect" mode: the edge sample is not repeated.
        let sample = |i: isize| -> f64 {
            let i = if i < 0 { -i } else if i as usize >= n { 2 * (n as isize - 1) - i } else { i };
            clip[i as usize] as f64
        };
        let frames = 1 + n / HOP;
        let mut out = Vec::with_capacity(frames * N_MELS);
        let mut buffer = vec![Complex::default(); N_FFT];
        for frame in 0..frames {
            let start = (frame * HOP) as isize - pad as isize;
            for (k, slot) in buffer.iter_mut().enumerate() {
                *slot = Complex::new(sample(start + k as isize) * self.window[k], 0.0);
            }
            self.fft.process(&mut buffer);
            let mut mel = [0.0; N_MELS];
            for (bin, weights) in self.filters.iter().enumerate() {
                // The reference keeps the spectrum as complex64.
                let (re, im) = (buffer[bin].re as f32 as f64, buffer[bin].im as f32 as f64);
                let power = re * re + im * im;
                for (m, w) in weights.iter().enumerate() {
                    mel[m] += w * power;
                }
            }
            out.extend(mel.iter().map(|&p| (10.0 * p.max(1e-10).log10()) as f32));
        }
        out
    }
}

fn hz_to_mel(hz: f64) -> f64 {
    if hz < 1000.0 {
        3.0 * hz / 200.0
    } else {
        15.0 + (hz / 1000.0).ln() * 27.0 / 6.4f64.ln()
    }
}

fn mel_to_hz(mel: f64) -> f64 {
    if mel < 15.0 {
        200.0 * mel / 3.0
    } else {
        1000.0 * (6.4f64.ln() / 27.0 * (mel - 15.0)).exp()
    }
}

/// Triangular Slaney-normalized filters on the Slaney mel scale.
fn mel_filters() -> Vec<[f64; N_MELS]> {
    let bins = N_FFT / 2 + 1;
    let (lo, hi) = (hz_to_mel(MEL_MIN_HZ), hz_to_mel(MEL_MAX_HZ));
    let edges: Vec<f64> = (0..N_MELS + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (N_MELS + 1) as f64)).collect();
    (0..bins)
        .map(|bin| {
            let freq = (SAMPLE_RATE / 2) as f64 * bin as f64 / (bins - 1) as f64;
            std::array::from_fn(|m| {
                let down = (freq - edges[m]) / (edges[m + 1] - edges[m]);
                let up = (edges[m + 2] - freq) / (edges[m + 2] - edges[m + 1]);
                down.min(up).max(0.0) * 2.0 / (edges[m + 2] - edges[m])
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mel_spectrogram_matches_the_reference_extractor() {
        // Values computed with transformers' ClapFeatureExtractor for the same signal.
        let clip: Vec<f32> = (0..CLIP_SAMPLES)
            .map(|n| {
                let t = n as f64 / SAMPLE_RATE as f64;
                let ramp = n as f64 / CLIP_SAMPLES as f64;
                (0.5 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()
                    + 0.25 * (2.0 * std::f64::consts::PI * 3000.0 * t).sin() * ramp) as f32
            })
            .collect();
        let mel = MelSpectrogram::new().compute(&clip);
        assert_eq!(mel.len(), FRAMES * N_MELS);
        let at = |frame: usize, m: usize| mel[frame * N_MELS + m] as f64;
        for (frame, m, expected) in [
            (0, 0, 8.627999305725098),
            (0, 10, 7.5672221183776855),
            (500, 5, 16.438762664794922),
            (500, 40, -91.99807739257812),
            (1000, 63, -36.186710357666016),
            (250, 20, -56.930137634277344),
        ] {
            assert!((at(frame, m) - expected).abs() < 0.01, "frame {frame} mel {m}: {} vs {expected}", at(frame, m));
        }
        let mean = mel.iter().map(|&x| x as f64).sum::<f64>() / mel.len() as f64;
        assert!((mean - -65.43701171875).abs() < 0.01, "mean {mean}");
    }

    #[test]
    fn probabilities_favour_the_closest_text() {
        let unit = |i: usize| -> Embedding { (0..4).map(|k| if k == i { 1.0 } else { 0.0 }).collect() };
        let sound: Vec<f32> = vec![0.8, 0.6, 0.0, 0.0];
        let p = probabilities(&sound, &[unit(0), unit(1), unit(2)]);
        assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        assert!(p[0] > p[1] && p[1] > p[2], "{p:?}");
    }

    /// End to end on the real model, when it is downloaded: run with
    /// `cargo test -- --ignored clap_model`.
    #[test]
    #[ignore]
    fn clap_model_tells_music_from_speech_like_noise() {
        let mut encoder = AudioEncoder::load().expect("model downloaded");
        let chord: Vec<f32> = (0..CLIP_SAMPLES)
            .map(|n| {
                let t = n as f64 / SAMPLE_RATE as f64;
                ([261.6, 329.6, 392.0].iter().map(|f| (2.0 * std::f64::consts::PI * f * t).sin()).sum::<f64>() * 0.2) as f32
            })
            .collect();
        let sound = encoder.embed(&chord).unwrap();
        assert_eq!(sound.len(), 512);
        let texts = text_embeddings(&["a sustained piano chord".to_owned(), "people talking".to_owned()]).unwrap();
        let p = probabilities(&sound, &texts);
        assert!(p[0] > p[1], "{p:?}");
    }
}
