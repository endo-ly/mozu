//! スピーカーへの再生。キューに積んだ音を、出力ストリームが順に取り出して鳴らす。
//! 合成しながら積めるので、長い文章でも最初の文ができた時点で鳴り始める。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, bail};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

pub struct Player {
    // 落とすとストリームが止まる。
    _stream: cpal::Stream,
    /// デバイスのサンプリング周波数にそろえたモノラルの音。
    queue: Arc<Mutex<VecDeque<f32>>>,
    sample_rate: u32,
}

impl Player {
    /// `device_hint` が空なら既定の出力デバイス、そうでなければ名前に含むデバイスを使う。
    pub fn open(device_hint: &str) -> anyhow::Result<Self> {
        let device = crate::device::output(device_hint)?;
        let supported = device
            .default_output_config()
            .context("出力デバイスの既定の設定を取得できません")?;
        let sample_rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let queue = Arc::new(Mutex::new(VecDeque::new()));

        let stream = match format {
            SampleFormat::I16 => build::<i16>(&device, &config, channels, &queue)?,
            SampleFormat::U16 => build::<u16>(&device, &config, channels, &queue)?,
            SampleFormat::I32 => build::<i32>(&device, &config, channels, &queue)?,
            SampleFormat::F32 => build::<f32>(&device, &config, channels, &queue)?,
            SampleFormat::F64 => build::<f64>(&device, &config, channels, &queue)?,
            other => bail!("対応していないサンプル形式です: {other}"),
        };
        stream.play().context("再生を開始できません")?;
        Ok(Self {
            _stream: stream,
            queue,
            sample_rate,
        })
    }

    /// 再生のキューに積む。`sample_rate` がデバイスと違えば変換する。
    pub fn push(&self, samples: &[f32], sample_rate: u32) {
        let samples = crate::resample(samples, sample_rate, self.sample_rate);
        self.lock().extend(samples);
    }

    /// 無音を積む（文と文の間など）。
    pub fn push_silence(&self, duration: Duration) {
        let n = (duration.as_secs_f64() * self.sample_rate as f64) as usize;
        self.lock().extend(std::iter::repeat_n(0.0, n));
    }

    /// まだ鳴らしていない音を捨てる。
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// 積んだ音を鳴らし終えるまで待つ。`stop` が立ったら残りを捨てて `false` を返す。
    pub fn wait(&self, stop: &AtomicBool) -> bool {
        loop {
            if stop.load(Ordering::Relaxed) {
                self.clear();
                return false;
            }
            if self.lock().is_empty() {
                // デバイス側のバッファに残っている分が鳴り終わるのを待つ
                std::thread::sleep(Duration::from_millis(150));
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<f32>> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    queue: &Arc<Mutex<VecDeque<f32>>>,
) -> anyhow::Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let queue = Arc::clone(queue);
    let stream = device.build_output_stream(
        *config,
        move |data: &mut [T], _: &_| {
            let mut queue = queue.lock().unwrap_or_else(|e| e.into_inner());
            for frame in data.chunks_mut(channels) {
                let s = T::from_sample(queue.pop_front().unwrap_or(0.0));
                frame.fill(s);
            }
        },
        |err| tracing::warn!("再生ストリームのエラー: {err}"),
        None,
    )?;
    Ok(stream)
}
