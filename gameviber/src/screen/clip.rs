//! CLIP image phase model (OpenAI `clip-vit-base-patch32`, quantized ONNX
//! export): copies of the game's image and short texts become vectors whose
//! similarity tells how well a text describes the image. Downloaded on demand
//! (`models::Model::Image`, about 150 MB) and run on the CPU.

use ort::session::Session;
use ort::value::Tensor;

use super::Frame;
use crate::models::{self, Embedding, Model};

/// `exp(logit_scale)` of the model: sharpness of the phase probabilities.
pub const LOGIT_SCALE: f64 = 100.0;
/// Side of the square the model looks at.
const SIZE: usize = 224;
const MEAN: [f32; 3] = [0.481_454_66, 0.457_827_5, 0.408_210_73];
// As `preprocessor_config.json` gives them.
#[allow(clippy::excessive_precision)]
const STD: [f32; 3] = [0.268_629_54, 0.261_302_58, 0.275_777_11];

pub struct ImageEncoder {
    session: Session,
}

impl ImageEncoder {
    pub fn load() -> anyhow::Result<Self> {
        Ok(Self { session: Model::Image.session("vision_model_quantized.onnx")? })
    }

    pub fn embed(&mut self, frame: &Frame) -> anyhow::Result<Embedding> {
        let pixels = preprocess(frame);
        let input = Tensor::from_array(([1usize, 3, SIZE, SIZE], pixels))?;
        let outputs = self.session.run(ort::inputs!["pixel_values" => input])?;
        let (_, embedding) = outputs["image_embeds"].try_extract_tensor::<f32>()?;
        Ok(models::normalize(embedding))
    }
}

/// The model's input, as `transformers`' `CLIPImageProcessor` makes it: the
/// shortest side resized to 224 with PIL's bicubic filter, the center
/// 224 x 224 cropped, then normalized; channels first.
fn preprocess(frame: &Frame) -> Vec<f32> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let scale = SIZE as f64 / w.min(h).max(1) as f64;
    let (rw, rh) = (((w as f64 * scale).round() as usize).max(SIZE), ((h as f64 * scale).round() as usize).max(SIZE));
    let rgb: Vec<u8> = frame.pixels.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    // Horizontal pass, then vertical: PIL's order, with 8-bit rounding in between.
    let wide = resample(&rgb, w, h, rw, true);
    let resized = resample(&wide, rw, h, rh, false);
    let (left, top) = ((rw - SIZE) / 2, (rh - SIZE) / 2);
    let mut out = vec![0f32; 3 * SIZE * SIZE];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = &resized[((top + y) * rw + left + x) * 3..][..3];
            for c in 0..3 {
                out[c * SIZE * SIZE + y * SIZE + x] = (p[c] as f32 / 255.0 - MEAN[c]) / STD[c];
            }
        }
    }
    out
}

fn bicubic(x: f64) -> f64 {
    const A: f64 = -0.5;
    let x = x.abs();
    if x < 1.0 {
        ((A + 2.0) * x - (A + 3.0)) * x * x + 1.0
    } else if x < 2.0 {
        (((x - 5.0) * x + 8.0) * x - 4.0) * A
    } else {
        0.0
    }
}

/// PIL's resampling of 8-bit RGB along one axis (`ImagingResample`): the
/// filter widens when shrinking, and fixed-point weights round like PIL's.
fn resample(src: &[u8], w: usize, h: usize, out_len: usize, horizontal: bool) -> Vec<u8> {
    const PRECISION: u32 = 22;
    let in_len = if horizontal { w } else { h };
    let scale = in_len as f64 / out_len as f64;
    let filter_scale = scale.max(1.0);
    let support = 2.0 * filter_scale;
    let taps: Vec<(usize, Vec<i64>)> = (0..out_len)
        .map(|i| {
            let center = (i as f64 + 0.5) * scale;
            let min = ((center - support + 0.5) as isize).max(0) as usize;
            let max = ((center + support + 0.5) as usize).min(in_len);
            let weights: Vec<f64> = (min..max).map(|x| bicubic((x as f64 - center + 0.5) / filter_scale)).collect();
            let total: f64 = weights.iter().sum();
            let fixed = weights
                .iter()
                .map(|w| {
                    let w = if total != 0.0 { w / total } else { 0.0 };
                    (w * (1u64 << PRECISION) as f64 + if w < 0.0 { -0.5 } else { 0.5 }) as i64
                })
                .collect();
            (min, fixed)
        })
        .collect();
    let (ow, oh) = if horizontal { (out_len, h) } else { (w, out_len) };
    let mut out = vec![0u8; ow * oh * 3];
    for y in 0..oh {
        for x in 0..ow {
            let (start, weights) = &taps[if horizontal { x } else { y }];
            for c in 0..3 {
                let mut sum: i64 = 1 << (PRECISION - 1);
                for (k, weight) in weights.iter().enumerate() {
                    let (sx, sy) = if horizontal { (start + k, y) } else { (x, start + k) };
                    sum += src[(sy * w + sx) * 3 + c] as i64 * weight;
                }
                out[(y * ow + x) * 3 + c] = (sum >> PRECISION).clamp(0, 255) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 480 x 270 test pattern with sharp edges, as the reference script builds it.
    fn pattern() -> Frame {
        let (w, h) = (480u32, 270u32);
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                pixels.extend([((x * 7 + y * 3) % 256) as u8, ((x ^ y) % 256) as u8, ((x * y / 7) % 256) as u8, 255]);
            }
        }
        Frame { width: w, height: h, source_width: 1920, source_height: 1080, count: 1, pixels }
    }

    /// Reference values from `CLIPImageProcessor`'s steps in Python (PIL bicubic resize,
    /// center crop, normalization) on the same pattern.
    #[test]
    fn preprocessing_matches_the_reference_processor() {
        let input = preprocess(&pattern());
        let at = |c: usize, y: usize, x: usize| input[c * SIZE * SIZE + y * SIZE + x];
        for (c, y, x, expected) in [
            (0, 0, 0, 1.46319),
            (0, 100, 50, 1.69676),
            (1, 223, 223, 0.09386),
            (2, 10, 200, -0.27151),
            (1, 57, 131, -0.74658),
            (2, 180, 3, -1.12472),
        ] {
            let got = at(c, y, x);
            // One 8-bit step is about 0.015 after normalization.
            assert!((got - expected).abs() < 0.02, "({c}, {y}, {x}): {got} instead of {expected}");
        }
        let mean = input.iter().sum::<f32>() / input.len() as f32;
        assert!((mean - 0.18075).abs() < 0.002, "mean {mean}");
    }

    /// End to end on the real model, when it is downloaded: run with
    /// `cargo test -- --ignored clip_model`.
    #[test]
    #[ignore]
    fn clip_model_matches_the_reference_embedding() {
        let mut encoder = ImageEncoder::load().expect("model downloaded");
        let image = encoder.embed(&pattern()).unwrap();
        assert_eq!(image.len(), 512);
        // Quantized kernels differ a little between ONNX Runtime versions (cosine 0.996 with Python's).
        for (got, expected) in image.iter().zip([0.0277, -0.0018, 0.0076, -0.0201]) {
            assert!((got - expected).abs() < 0.012, "{:?}", &image[..4]);
        }
        let texts = models::text_embeddings(Model::Image, &["a colorful abstract pattern".to_owned(), "a photo of a dog".to_owned()]).unwrap();
        let p = models::probabilities(&image, &texts, LOGIT_SCALE);
        assert!(p[0] > p[1], "{p:?}");
    }
}
