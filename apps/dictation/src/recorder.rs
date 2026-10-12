//! マイク録音。録音中だけ入力ストリームを開き、モノラルの f32 にまとめて溜める。

use std::io::Cursor;
use std::sync::{Arc, Mutex};

use anyhow::{Context, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

pub struct Recorder {
    // 落とすとストリームが止まる。
    _stream: cpal::Stream,
    buffer: Arc<Mutex<Buffer>>,
    sample_rate: u32,
}

struct Buffer {
    samples: Vec<f32>,
    max_samples: usize,
    truncated: bool,
}

pub struct Recording {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub truncated: bool,
}

impl Recording {
    pub fn seconds(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate as f32
    }

    /// 16bit PCM モノラルの WAV にする。サンプリング周波数はデバイスのまま
    /// （speech.cpp 側で 16kHz に変換される）。
    pub fn to_wav(&self) -> anyhow::Result<Vec<u8>> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: self.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = Cursor::new(Vec::with_capacity(44 + self.samples.len() * 2));
        let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
        for &s in &self.samples {
            writer.write_sample(i16::from_sample(s.clamp(-1.0, 1.0)))?;
        }
        writer.finalize()?;
        Ok(cursor.into_inner())
    }
}

impl Recorder {
    /// `device_hint` が空なら既定の入力デバイス、そうでなければ名前に含むデバイスを使う。
    pub fn start(device_hint: &str, max_secs: u32) -> anyhow::Result<Self> {
        let device = find_device(device_hint)?;
        let supported = device
            .default_input_config()
            .context("入力デバイスの既定の設定を取得できません")?;
        let sample_rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();

        let buffer = Arc::new(Mutex::new(Buffer {
            samples: Vec::with_capacity(sample_rate as usize * 30),
            max_samples: sample_rate as usize * max_secs as usize,
            truncated: false,
        }));

        let stream = match format {
            SampleFormat::I16 => build::<i16>(&device, &config, channels, &buffer)?,
            SampleFormat::U16 => build::<u16>(&device, &config, channels, &buffer)?,
            SampleFormat::I32 => build::<i32>(&device, &config, channels, &buffer)?,
            SampleFormat::F32 => build::<f32>(&device, &config, channels, &buffer)?,
            SampleFormat::F64 => build::<f64>(&device, &config, channels, &buffer)?,
            other => bail!("対応していないサンプル形式です: {other}"),
        };
        stream.play().context("録音を開始できません")?;

        Ok(Self {
            _stream: stream,
            buffer,
            sample_rate,
        })
    }

    pub fn stop(self) -> Recording {
        let sample_rate = self.sample_rate;
        let buffer = Arc::clone(&self.buffer);
        drop(self); // ストリームを先に止めてから中身を取り出す
        let mut buffer = buffer.lock().unwrap_or_else(|e| e.into_inner());
        Recording {
            samples: std::mem::take(&mut buffer.samples),
            sample_rate,
            truncated: buffer.truncated,
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    buffer: &Arc<Mutex<Buffer>>,
) -> anyhow::Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let buffer = Arc::clone(buffer);
    let stream = device.build_input_stream(
        *config,
        move |data: &[T], _: &_| {
            let mut buffer = buffer.lock().unwrap_or_else(|e| e.into_inner());
            for frame in data.chunks(channels) {
                if buffer.samples.len() >= buffer.max_samples {
                    buffer.truncated = true;
                    return;
                }
                let sum: f32 = frame.iter().map(|s| s.to_sample::<f32>()).sum();
                buffer.samples.push(sum / frame.len() as f32);
            }
        },
        |err| tracing::warn!("録音ストリームのエラー: {err}"),
        None,
    )?;
    Ok(stream)
}

fn find_device(hint: &str) -> anyhow::Result<cpal::Device> {
    let host = cpal::default_host();
    if hint.is_empty() {
        return host
            .default_input_device()
            .ok_or_else(|| anyhow!("既定の入力デバイスがありません"));
    }
    let hint = hint.to_lowercase();
    host.input_devices()?
        .find(|d| {
            d.description()
                .is_ok_and(|desc| desc.to_string().to_lowercase().contains(&hint))
        })
        .ok_or_else(|| anyhow!("名前に「{hint}」を含む入力デバイスがありません"))
}

/// `--list-devices` 用。入力デバイスの名前の一覧。
pub fn list_devices() -> anyhow::Result<Vec<String>> {
    let host = cpal::default_host();
    let default = host
        .default_input_device()
        .and_then(|d| d.description().ok())
        .map(|d| d.to_string());
    Ok(host
        .input_devices()?
        .filter_map(|d| d.description().ok().map(|d| d.to_string()))
        .map(|name| {
            if default.as_deref() == Some(name.as_str()) {
                format!("{name}  (既定)")
            } else {
                name
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 既定のマイクから 1 秒録って WAV にする。
    /// `cargo test -p mozu-dictation -- --ignored` で実行する。
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
        let reader = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, recording.sample_rate);
    }
}
