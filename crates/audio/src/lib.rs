//! マイクの録音とスピーカーへの再生（cpal / WASAPI）、WAV の読み書き。
//!
//! 音声はどこでもモノラルの f32（-1.0〜1.0）で扱う。

mod device;
mod play;
mod record;
pub mod wav;

pub use device::{list_inputs, list_outputs};
pub use play::Player;
pub use record::{Recorder, Recording};

/// 線形補間でサンプリング周波数を変える。話し声の再生には十分な品質。
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let len = (samples.len() as f64 / ratio).round() as usize;
    (0..len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let j = pos as usize;
            let frac = (pos - j as f64) as f32;
            let a = samples[j.min(samples.len() - 1)];
            let b = samples[(j + 1).min(samples.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

/// 二乗平均平方根（音の大きさの目安）。
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_changes_length() {
        let samples: Vec<f32> = (0..480).map(|i| i as f32 / 480.0).collect();
        let out = resample(&samples, 48_000, 16_000);
        assert_eq!(out.len(), 160);
        assert!((out[80] - samples[240]).abs() < 1e-3);
    }

    /// 既定のマイクから 1 秒録って WAV にする。
    /// `cargo test -p mozu-audio -- --ignored` で実行する。
    #[test]
    #[ignore = "マイクが必要"]
    fn record_from_default_device() {
        let recorder = Recorder::start("", 10).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(1));
        let recording = recorder.stop();
        assert!(
            recording.seconds() > 0.8,
            "{} 秒しか録れていない",
            recording.seconds()
        );
        let wav = recording.to_wav().unwrap();
        let (samples, rate) = wav::decode(&wav).unwrap();
        assert_eq!(rate, recording.sample_rate);
        assert_eq!(samples.len(), recording.samples.len());
    }

    /// 既定のスピーカーで 0.3 秒の音を鳴らす。
    #[test]
    #[ignore = "スピーカーが必要"]
    fn play_tone() {
        let player = Player::open("").unwrap();
        let tone: Vec<f32> = (0..14_400)
            .map(|i| (i as f32 / 48_000.0 * 440.0 * std::f32::consts::TAU).sin() * 0.1)
            .collect();
        player.push(&tone, 48_000);
        let stop = std::sync::atomic::AtomicBool::new(false);
        assert!(player.wait(&stop));
    }
}
