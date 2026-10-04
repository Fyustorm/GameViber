//! Cheap measures of the game's sound, every 20 ms: loudness overall and per
//! band, a slow intensity, and hits (sudden attacks, found by spectral flux).
//! Levels are relative to the game's recent loudest moments, so they do not
//! depend on its volume setting.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use serde::{Deserialize, Serialize};

use super::SAMPLE_RATE;

/// Samples between two analyses: 20 ms, one engine tick.
pub const HOP: usize = 960;
const FFT_SIZE: usize = 2048;
const HOP_SECS: f64 = HOP as f64 / SAMPLE_RATE as f64;
/// Upper edges of the low and mid bands, in Hz; high goes up to 16 kHz.
const LOW_MAX: f64 = 250.0;
const MID_MAX: f64 = 4000.0;
const HIGH_MAX: f64 = 16000.0;
/// Levels span this many dB below the recent loudest moment.
const RANGE_DB: f64 = 40.0;
/// Below this, the sound counts as silence.
const SILENCE_DB: f64 = -70.0;
/// The loudness reference never goes below this, so that quiet passages stay quiet.
const REF_FLOOR_DB: f64 = -45.0;
/// How fast the loudness reference forgets a loud moment, in dB per second.
const REF_DECAY: f64 = 0.5;
/// Time constant of `intensity`, in seconds.
const INTENSITY_SECS: f64 = 6.0;
/// Hits per second that make the hit part of `intensity` full.
const BUSY_HITS_PER_SEC: f64 = 3.0;
/// Flux statistics follow the last ~1 s.
const FLUX_STATS_SECS: f64 = 1.0;
/// A hit stands this many deviations above the average flux.
const HIT_DEVIATIONS: f64 = 4.0;
const MIN_HIT_GAP: f64 = 0.08;
/// The flux of the strongest recent hit of each band decays by this factor per second.
const PEAK_DECAY: f64 = 0.97;
/// No hits while the flux statistics settle, after the analysis starts.
const WARMUP_SECS: f64 = 0.5;

/// The game's sound right now, each 0..1.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioLevels {
    pub level: f64,
    pub low: f64,
    pub mid: f64,
    pub high: f64,
    /// Loudness and density of hits over the last ~6 s.
    pub intensity: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Band {
    Low,
    Mid,
    High,
}

impl Band {
    pub fn name(self) -> &'static str {
        match self {
            Band::Low => "low",
            Band::Mid => "mid",
            Band::High => "high",
        }
    }
}

/// A sudden attack in the sound: an impact, a shot, an explosion...
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AudioHit {
    /// 0..1, relative to the strongest recent hits.
    pub strength: f64,
    /// The band where the attack is strongest.
    pub band: Band,
}

/// Loudness reference following the recent loudest moments.
struct Reference(f64);

impl Reference {
    fn normalize(&mut self, db: f64) -> f64 {
        self.0 = (self.0 - REF_DECAY * HOP_SECS).max(db).max(REF_FLOOR_DB);
        if db < SILENCE_DB {
            return 0.0;
        }
        (1.0 + (db - self.0) / RANGE_DB).clamp(0.0, 1.0)
    }
}

pub struct Analyzer {
    fft: Arc<dyn Fft<f64>>,
    window: Vec<f64>,
    /// The last `FFT_SIZE` samples.
    buffer: Vec<f32>,
    /// Samples received since the last analysis.
    pending: usize,
    spectrum: Vec<Complex<f64>>,
    /// Log-compressed magnitudes of the previous frame, for the flux.
    previous: Vec<f64>,
    bands: [std::ops::Range<usize>; 3],
    refs: [Reference; 4],
    intensity_level: f64,
    intensity_hits: f64,
    flux_mean: f64,
    flux_dev: f64,
    /// Flux of the last two frames, for peak picking.
    flux_history: [(f64, [f64; 3]); 2],
    /// Flux of the strongest recent hit, per band.
    peak_flux: [f64; 3],
    since_hit: f64,
    /// Seconds analysed so far.
    elapsed: f64,
    levels: AudioLevels,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    pub fn new() -> Self {
        let bin = |hz: f64| ((hz * FFT_SIZE as f64 / SAMPLE_RATE as f64).round() as usize).min(FFT_SIZE / 2);
        Self {
            fft: FftPlanner::new().plan_fft_forward(FFT_SIZE),
            window: (0..FFT_SIZE).map(|n| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / FFT_SIZE as f64).cos()).collect(),
            buffer: vec![0.0; FFT_SIZE],
            pending: 0,
            spectrum: vec![Complex::default(); FFT_SIZE],
            previous: vec![0.0; FFT_SIZE / 2 + 1],
            bands: [bin(20.0)..bin(LOW_MAX), bin(LOW_MAX)..bin(MID_MAX), bin(MID_MAX)..bin(HIGH_MAX)],
            refs: std::array::from_fn(|_| Reference(REF_FLOOR_DB)),
            intensity_level: 0.0,
            intensity_hits: 0.0,
            flux_mean: 0.0,
            flux_dev: 0.0,
            flux_history: [(0.0, [0.0; 3]); 2],
            peak_flux: [0.0; 3],
            since_hit: f64::INFINITY,
            elapsed: 0.0,
            levels: AudioLevels::default(),
        }
    }

    /// Feeds mono samples at `SAMPLE_RATE`; `on_frame` is called every `HOP`
    /// samples with the levels and the hit found at that frame, if any.
    pub fn push(&mut self, samples: &[f32], mut on_frame: impl FnMut(AudioLevels, Option<AudioHit>)) {
        let mut rest = samples;
        while !rest.is_empty() {
            let take = rest.len().min(HOP - self.pending);
            self.buffer.drain(..take);
            self.buffer.extend_from_slice(&rest[..take]);
            self.pending += take;
            rest = &rest[take..];
            if self.pending == HOP {
                self.pending = 0;
                let hit = self.analyze();
                on_frame(self.levels, hit);
            }
        }
    }

    fn analyze(&mut self) -> Option<AudioHit> {
        let hop = &self.buffer[FFT_SIZE - HOP..];
        let rms = (hop.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>() / HOP as f64).sqrt();
        let db = 20.0 * rms.max(1e-10).log10();

        for (i, (x, w)) in self.buffer.iter().zip(&self.window).enumerate() {
            self.spectrum[i] = Complex::new(*x as f64 * w, 0.0);
        }
        self.fft.process(&mut self.spectrum);
        // Window gain: a full-scale sine reads about 0 dB in its band.
        let scale = 2.0 / self.window.iter().sum::<f64>();
        let mut band_db = [0.0; 3];
        let mut flux_per_bin = [0.0; 3];
        for (b, range) in self.bands.iter().enumerate() {
            let mut power = 0.0;
            for k in range.clone() {
                let magnitude = self.spectrum[k].norm() * scale;
                power += magnitude * magnitude;
                let compressed = (1.0 + 100.0 * magnitude).ln();
                flux_per_bin[b] += (compressed - self.previous[k]).max(0.0);
                self.previous[k] = compressed;
            }
            flux_per_bin[b] /= range.len().max(1) as f64;
            // Half the power of the sines in the band, like the RMS of a sine.
            band_db[b] = 10.0 * (power / 2.0).max(1e-20).log10();
        }

        let [level, low, mid, high] = &mut self.refs;
        let level = level.normalize(db);
        let (low, mid, high) = (low.normalize(band_db[0]), mid.normalize(band_db[1]), high.normalize(band_db[2]));
        let hit = self.detect_hit(flux_per_bin, db);
        let alpha = HOP_SECS / INTENSITY_SECS;
        self.intensity_level += alpha * (level - self.intensity_level);
        let rate = if hit.is_some() { 1.0 / HOP_SECS } else { 0.0 };
        self.intensity_hits += alpha * (rate - self.intensity_hits);
        let intensity = (0.7 * self.intensity_level + 0.3 * (self.intensity_hits / BUSY_HITS_PER_SEC).min(1.0)).clamp(0.0, 1.0);
        self.levels = AudioLevels { level, low, mid, high, intensity };
        hit
    }

    /// A hit is a local peak of the flux well above its recent average. It is
    /// reported one frame late, once the peak is known; its band is the one
    /// whose bins rose the most. The flux is averaged per bin in each band, so
    /// that the narrow low band weighs as much as the others.
    fn detect_hit(&mut self, flux_per_bin: [f64; 3], db: f64) -> Option<AudioHit> {
        let total: f64 = flux_per_bin.iter().sum();
        self.elapsed += HOP_SECS;
        let [(before, _), (candidate, bands)] = self.flux_history;
        self.flux_history = [self.flux_history[1], (total, flux_per_bin)];
        self.since_hit += HOP_SECS;
        let decay = PEAK_DECAY.powf(HOP_SECS);
        self.peak_flux.iter_mut().for_each(|p| *p *= decay);

        let threshold = self.flux_mean + HIT_DEVIATIONS * self.flux_dev.max(0.05 * self.flux_mean);
        let alpha = HOP_SECS / FLUX_STATS_SECS;
        self.flux_dev += alpha * ((total - self.flux_mean).abs() - self.flux_dev);
        self.flux_mean += alpha * (total - self.flux_mean);

        let is_peak = candidate > before && candidate >= total && candidate > threshold;
        if !is_peak || self.elapsed < WARMUP_SECS || self.since_hit < MIN_HIT_GAP || db < SILENCE_DB || threshold <= 0.0 {
            return None;
        }
        self.since_hit = 0.0;
        let b = (0..3).max_by(|&x, &y| bands[x].total_cmp(&bands[y])).unwrap_or(1);
        self.peak_flux[b] = self.peak_flux[b].max(candidate);
        let strength = ((candidate - threshold) / (self.peak_flux[b] - threshold).max(1e-9)).clamp(0.05, 1.0);
        Some(AudioHit { strength, band: [Band::Low, Band::Mid, Band::High][b] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, amplitude: f64, secs: f64) -> Vec<f32> {
        let n = (secs * SAMPLE_RATE as f64) as usize;
        (0..n).map(|i| (amplitude * (2.0 * std::f64::consts::PI * freq * i as f64 / SAMPLE_RATE as f64).sin()) as f32).collect()
    }

    /// Runs the analyzer, returning the last levels and the time of every hit.
    fn run(analyzer: &mut Analyzer, samples: &[f32]) -> (AudioLevels, Vec<(f64, AudioHit)>) {
        let mut hits = Vec::new();
        let mut frames = 0;
        let mut last = AudioLevels::default();
        analyzer.push(samples, |levels, hit| {
            frames += 1;
            last = levels;
            if let Some(hit) = hit {
                hits.push((frames as f64 * HOP_SECS, hit));
            }
        });
        (last, hits)
    }

    #[test]
    fn steady_sound_has_levels_in_its_band_and_no_hits() {
        let mut analyzer = Analyzer::new();
        let (levels, hits) = run(&mut analyzer, &sine(100.0, 0.5, 3.0));
        assert!(hits.is_empty(), "{hits:?}");
        assert!(levels.level > 0.9, "{levels:?}");
        assert!(levels.low > 0.9, "{levels:?}");
        assert!(levels.high < 0.1, "{levels:?}");

        let (levels, _) = run(&mut analyzer, &sine(8000.0, 0.5, 3.0));
        assert!(levels.high > 0.9 && levels.low < 0.1, "{levels:?}");
    }

    #[test]
    fn quieter_passages_read_lower_until_the_reference_adapts() {
        let mut analyzer = Analyzer::new();
        run(&mut analyzer, &sine(440.0, 0.5, 2.0));
        // 20 dB quieter: half of the 40 dB range.
        let (levels, _) = run(&mut analyzer, &sine(440.0, 0.05, 0.5));
        assert!((levels.level - 0.5).abs() < 0.05, "{levels:?}");
        // Silence is 0.
        let (levels, _) = run(&mut analyzer, &vec![0.0; SAMPLE_RATE as usize]);
        assert_eq!(levels.level, 0.0);
    }

    #[test]
    fn bursts_over_a_quiet_background_are_hits() {
        let mut samples = sine(440.0, 0.02, 4.0);
        // 60 ms tone bursts: high, low, high.
        for (start, freq) in [(1.0, 6000.0), (2.0, 60.0), (3.0, 6000.0)] {
            let at = (start * SAMPLE_RATE as f64) as usize;
            for (x, b) in samples[at..].iter_mut().zip(sine(freq, 0.8, 0.06)) {
                *x += b;
            }
        }
        let mut analyzer = Analyzer::new();
        let (levels, hits) = run(&mut analyzer, &samples);
        let times: Vec<f64> = hits.iter().map(|(t, _)| *t).collect();
        assert_eq!(times.len(), 3, "{hits:?}");
        for (t, expected) in times.iter().zip([1.0, 2.0, 3.0]) {
            assert!((t - expected).abs() < 0.08, "{times:?}");
        }
        let bands: Vec<Band> = hits.iter().map(|(_, h)| h.band).collect();
        assert_eq!(bands, [Band::High, Band::Low, Band::High]);
        assert!(hits.iter().all(|(_, h)| h.strength > 0.3), "{hits:?}");
        assert!(levels.intensity > 0.0);
    }
}
