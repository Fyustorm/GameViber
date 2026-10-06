//! Models downloaded on demand and run on the CPU with ONNX Runtime: the
//! sound scene model (CLAP, `audio::clap`) and the image scene model (CLIP,
//! `screen::clip`). Each is fetched from a pinned revision of its repository,
//! so the files never change under us.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context};
use ort::session::Session;

use crate::config;

/// Threads each model may use: the game needs the CPU more.
const THREADS: usize = 2;

/// A unit vector describing a sound, an image or a text.
pub type Embedding = Arc<[f32]>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Model {
    /// LAION `larger_clap_music_and_speech`: sound and text.
    Sound,
    /// OpenAI `clip-vit-base-patch32`: image and text.
    Image,
}

struct Files {
    dir: &'static str,
    repo: &'static str,
    revision: &'static str,
    /// Repository path, local name, size in bytes.
    files: &'static [(&'static str, &'static str, u64)],
}

const SOUND: Files = Files {
    dir: "clap-music-speech",
    repo: "https://huggingface.co/Xenova/larger_clap_music_and_speech/resolve",
    revision: "e9fd5ac1dbf3280936a7fc3ec8a020453ff184db",
    files: &[
        ("onnx/audio_model_quantized.onnx", "audio_model_quantized.onnx", 78_155_433),
        ("onnx/text_model_quantized.onnx", "text_model_quantized.onnx", 126_603_262),
        ("tokenizer.json", "tokenizer.json", 2_108_774),
    ],
};

const IMAGE: Files = Files {
    dir: "clip-vit-base-patch32",
    repo: "https://huggingface.co/Xenova/clip-vit-base-patch32/resolve",
    revision: "d15189d7028b43f1d3e65039190477f6af591c2a",
    files: &[
        ("onnx/vision_model_quantized.onnx", "vision_model_quantized.onnx", 89_117_001),
        ("onnx/text_model_quantized.onnx", "text_model_quantized.onnx", 64_504_507),
        ("tokenizer.json", "tokenizer.json", 2_224_119),
    ],
};

#[derive(Debug, Clone, Default, PartialEq)]
pub enum ModelState {
    #[default]
    Missing,
    Downloading { done: u64, total: u64 },
    Ready,
    Failed(String),
}

static STATES: Mutex<Option<HashMap<Model, ModelState>>> = Mutex::new(None);

impl Model {
    fn files(self) -> &'static Files {
        match self {
            Model::Sound => &SOUND,
            Model::Image => &IMAGE,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Model::Sound => "sound phase model",
            Model::Image => "image phase model",
        }
    }

    pub fn dir(self) -> PathBuf {
        config::data_dir().join("models").join(self.files().dir)
    }

    pub fn path(self, name: &str) -> PathBuf {
        self.dir().join(name)
    }

    /// Bytes to download.
    /// Longest token sequence its text encoder takes.
    fn max_tokens(self) -> usize {
        match self {
            Model::Sound => 512,
            Model::Image => 77,
        }
    }

    pub fn size(self) -> u64 {
        self.files().files.iter().map(|(_, _, size)| size).sum()
    }

    fn present(self) -> bool {
        let dir = self.dir();
        self.files().files.iter().all(|(_, name, size)| fs::metadata(dir.join(name)).is_ok_and(|m| m.len() == *size))
    }

    pub fn state(self) -> ModelState {
        let mut states = STATES.lock().unwrap();
        let states = states.get_or_insert_with(HashMap::new);
        states.entry(self).or_insert_with(|| if self.present() { ModelState::Ready } else { ModelState::Missing }).clone()
    }

    pub fn ready(self) -> bool {
        self.state() == ModelState::Ready
    }

    fn set_state(self, state: ModelState) {
        STATES.lock().unwrap().get_or_insert_with(HashMap::new).insert(self, state);
    }

    /// Downloads the model in the background; `state()` follows the progress.
    pub fn start_download(self) {
        if matches!(self.state(), ModelState::Ready | ModelState::Downloading { .. }) {
            return;
        }
        self.set_state(ModelState::Downloading { done: 0, total: self.size() });
        std::thread::Builder::new()
            .name("model-download".into())
            .spawn(move || match self.download() {
                Ok(()) => {
                    log::info!("{} downloaded to {}", self.label(), self.dir().display());
                    self.set_state(ModelState::Ready);
                }
                Err(e) => {
                    log::error!("{} download failed: {e:#}", self.label());
                    self.set_state(ModelState::Failed(format!("{e:#}")));
                }
            })
            .expect("spawn the download thread");
    }

    fn download(self) -> anyhow::Result<()> {
        let files = self.files();
        let dir = self.dir();
        config::create_dir(&dir)?;
        let total = self.size();
        let mut done = 0;
        for &(path, name, size) in files.files {
            let target = dir.join(name);
            if fs::metadata(&target).is_ok_and(|m| m.len() == size) {
                done += size;
                continue;
            }
            let url = format!("{}/{}/{path}", files.repo, files.revision);
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
                self.set_state(ModelState::Downloading { done: done + received, total });
            }
            file.flush()?;
            anyhow::ensure!(received == size, "{name}: got {received} bytes instead of {size}");
            fs::rename(&partial, &target)?;
            done += size;
        }
        Ok(())
    }

    /// An ONNX Runtime session for one of the model's files.
    pub fn session(self, name: &str) -> anyhow::Result<Session> {
        let path = self.path(name);
        let builder = Session::builder()?;
        let builder = builder.with_intra_threads(THREADS).map_err(|e| anyhow!("{e}"))?;
        builder
            .with_inter_threads(1)
            .map_err(|e| anyhow!("{e}"))?
            .commit_from_file(&path)
            .map_err(|e| anyhow!("loading {}: {e}", path.display()))
    }
}

pub fn normalize(v: &[f32]) -> Embedding {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    v.iter().map(|x| x / norm).collect()
}

pub fn dot(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x * y) as f64).sum()
}

/// Softmax of `scale` times the similarity of `v` with each of `refs`.
pub fn probabilities(v: &[f32], refs: &[Embedding], scale: f64) -> Vec<f64> {
    let logits: Vec<f64> = refs.iter().map(|r| scale * dot(r, v)).collect();
    let max = logits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let exp: Vec<f64> = logits.iter().map(|l| (l - max).exp()).collect();
    let sum: f64 = exp.iter().sum();
    exp.into_iter().map(|e| e / sum).collect()
}

/// Text embeddings of a model's text encoder, computed once per text; the
/// text model is only loaded while new texts are encoded. `input_ids` is the
/// only input of both text models.
pub fn text_embeddings(model: Model, texts: &[String]) -> anyhow::Result<Vec<Embedding>> {
    static CACHE: Mutex<Option<HashMap<(Model, String), Embedding>>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);
    let missing: Vec<&String> = texts.iter().filter(|t| !cache.contains_key(&(model, (*t).clone()))).collect();
    if !missing.is_empty() {
        anyhow::ensure!(model.ready(), "the {} is not downloaded", model.label());
        let tokenizer = tokenizers::Tokenizer::from_file(model.path("tokenizer.json")).map_err(|e| anyhow!("tokenizer: {e}"))?;
        let mut session = model.session("text_model_quantized.onnx")?;
        for text in missing {
            let encoding = tokenizer.encode(text.as_str(), true).map_err(|e| anyhow!("tokenizer: {e}"))?;
            let mut ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
            if ids.len() > model.max_tokens() {
                // Keep the end-of-text token, which the encoders pool on.
                let last = ids[ids.len() - 1];
                ids.truncate(model.max_tokens() - 1);
                ids.push(last);
            }
            let input = ort::value::Tensor::from_array(([1usize, ids.len()], ids))?;
            let outputs = session.run(ort::inputs!["input_ids" => input])?;
            let (_, embedding) = outputs["text_embeds"].try_extract_tensor::<f32>()?;
            cache.insert((model, text.clone()), normalize(embedding));
        }
    }
    Ok(texts.iter().map(|t| cache[&(model, t.clone())].clone()).collect())
}
