//! A small energy-based voice-activity detector.
//!
//! Readback's core never touches audio: the host supplies speech regions. This
//! module is what a host would plug in, kept deliberately simple so the demo has
//! no model to download. It is short-time energy with hysteresis, which is
//! enough to show omission detection working on a real clip and nowhere near
//! good enough to ship in a noisy room — use Silero or WebRTC VAD for that.

use readback_core::SpeechRegion;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VadConfig {
    /// Analysis window. 30 ms is the usual choice for speech.
    pub frame_ms: u32,
    /// A frame counts as speech when its RMS is this many times the noise floor.
    pub threshold_ratio: f32,
    /// Speech shorter than this is discarded as a click or a breath.
    pub min_speech_ms: u32,
    /// Silence shorter than this does not split a region in two.
    pub min_silence_ms: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self { frame_ms: 30, threshold_ratio: 2.5, min_speech_ms: 90, min_silence_ms: 120 }
    }
}

/// Mono samples in `-1.0..=1.0`, plus the sample rate they were taken at.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl Clip {
    pub fn duration_ms(&self) -> u32 {
        if self.sample_rate == 0 {
            return 0;
        }
        (self.samples.len() as u64 * 1000 / self.sample_rate as u64) as u32
    }
}

fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
}

/// The noise floor, taken as the 20th percentile of frame energy.
///
/// A percentile rather than the minimum, because one silent frame in a clip
/// that is otherwise all speech would drive the threshold to zero.
fn noise_floor(energies: &[f32]) -> f32 {
    let mut sorted = energies.to_vec();
    sorted.sort_by(f32::total_cmp);
    let index = sorted.len() / 5;
    sorted.get(index).copied().unwrap_or(0.0)
}

/// Finds the stretches of a clip that contain speech.
pub fn detect(clip: &Clip, cfg: &VadConfig) -> Vec<SpeechRegion> {
    if clip.samples.is_empty() || clip.sample_rate == 0 {
        return Vec::new();
    }

    let frame_len = ((clip.sample_rate as u64 * cfg.frame_ms as u64) / 1000).max(1) as usize;
    let energies: Vec<f32> = clip.samples.chunks(frame_len).map(rms).collect();
    if energies.is_empty() {
        return Vec::new();
    }

    let floor = noise_floor(&energies);
    // An absolute floor as well, so a clip of pure silence does not become all
    // speech just because its own noise floor is tiny.
    let threshold = (floor * cfg.threshold_ratio).max(0.005);

    let frame_ms = (frame_len as u64 * 1000 / clip.sample_rate as u64) as u32;
    let min_silence_frames = cfg.min_silence_ms.div_ceil(frame_ms.max(1));

    let mut regions: Vec<(usize, usize)> = Vec::new();
    let mut current: Option<(usize, usize)> = None;
    let mut silence_run = 0usize;

    for (i, energy) in energies.iter().enumerate() {
        if *energy >= threshold {
            silence_run = 0;
            match &mut current {
                Some((_, end)) => *end = i + 1,
                None => current = Some((i, i + 1)),
            }
        } else if current.is_some() {
            silence_run += 1;
            if silence_run as u32 >= min_silence_frames {
                regions.push(current.take().expect("current is some"));
                silence_run = 0;
            }
        }
    }
    if let Some(region) = current {
        regions.push(region);
    }

    regions
        .into_iter()
        .map(|(start, end)| {
            SpeechRegion::new(start as u32 * frame_ms, (end as u32 * frame_ms).min(clip.duration_ms()))
        })
        .filter(|r| r.duration_ms() >= cfg.min_speech_ms)
        .collect()
}

/// Reads a WAV file and downmixes it to mono `f32`.
pub fn read_wav(path: &std::path::Path) -> Result<Clip, String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| format!("opening {}: {e}", path.display()))?;
    let spec = reader.spec();

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|s| s as f32 * scale)
                .collect()
        }
    };

    let channels = spec.channels.max(1) as usize;
    let mono = if channels == 1 {
        samples
    } else {
        samples
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    };

    Ok(Clip { samples: mono, sample_rate: spec.sample_rate })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a clip: a list of (duration_ms, amplitude) segments at 16 kHz.
    fn clip(segments: &[(u32, f32)]) -> Clip {
        let sample_rate = 16_000;
        let mut samples = Vec::new();
        let mut phase = 0.0f32;
        for (duration_ms, amplitude) in segments {
            let count = (sample_rate as u64 * *duration_ms as u64 / 1000) as usize;
            for _ in 0..count {
                phase += 0.2;
                samples.push(phase.sin() * amplitude);
            }
        }
        Clip { samples, sample_rate }
    }

    #[test]
    fn silence_has_no_speech() {
        assert!(detect(&clip(&[(1000, 0.0)]), &VadConfig::default()).is_empty());
    }

    #[test]
    fn an_empty_clip_is_handled() {
        let empty = Clip { samples: Vec::new(), sample_rate: 16_000 };
        assert!(detect(&empty, &VadConfig::default()).is_empty());
    }

    #[test]
    fn one_utterance_becomes_one_region() {
        let c = clip(&[(300, 0.0), (500, 0.4), (300, 0.0)]);
        let regions = detect(&c, &VadConfig::default());
        assert_eq!(regions.len(), 1);
        assert!(regions[0].start_ms >= 240 && regions[0].start_ms <= 360, "{regions:?}");
        assert!(regions[0].duration_ms() >= 400, "{regions:?}");
    }

    #[test]
    fn a_long_pause_splits_two_utterances() {
        let c = clip(&[(200, 0.0), (400, 0.4), (500, 0.0), (400, 0.4), (200, 0.0)]);
        assert_eq!(detect(&c, &VadConfig::default()).len(), 2);
    }

    #[test]
    fn a_short_pause_does_not() {
        let c = clip(&[(200, 0.0), (400, 0.4), (60, 0.0), (400, 0.4), (200, 0.0)]);
        assert_eq!(detect(&c, &VadConfig::default()).len(), 1);
    }

    #[test]
    fn a_click_is_not_speech() {
        let c = clip(&[(500, 0.0), (20, 0.5), (500, 0.0)]);
        assert!(detect(&c, &VadConfig::default()).is_empty());
    }

    #[test]
    fn duration_is_reported_in_milliseconds() {
        assert_eq!(clip(&[(1500, 0.1)]).duration_ms(), 1500);
    }
}
