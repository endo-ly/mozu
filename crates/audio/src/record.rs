//! マイク録音。録音中だけ入力ストリームを開き、モノラルの f32 にまとめて溜める。

use std::sync::{Arc, Mutex};

use anyhow::{Context, bail};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

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
        crate::wav::encode(&self.samples, self.sample_rate)
    }
}

impl Recorder {
    /// `device_hint` が空なら既定の入力デバイス、そうでなければ名前に含むデバイスを使う。
    pub fn start(device_hint: &str, max_secs: u32) -> anyhow::Result<Self> {
        let device = crate::device::input(device_hint)?;
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

    /// 録音中の音声のうち、`from` 番目以降の写し（無音の検出などに使う）。
    pub fn peek(&self, from: usize) -> Vec<f32> {
        let buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
        buffer
            .samples
            .get(from..)
            .map(<[f32]>::to_vec)
            .unwrap_or_default()
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
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
