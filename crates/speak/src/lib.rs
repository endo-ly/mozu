//! 文章の読み上げ。下ごしらえ → 合成（speech.cpp）→ 再生（mozu-audio）。
//!
//! 下ごしらえは、Markdown の記号・URL・コードの塊を取り除き（`clean`）、英単語をカタカナにする
//! （`Readings`。Irodori-TTS は英語の読みが崩れるため）。
//!
//! 文章は丸ごと 1 回で送る。speech.cpp が文ごとに合成し（どの文も同じ種）、文と文の間を空けて、
//! できた文から順に返してくるので、届いた分から鳴らす。長い文章でも、最初の文ができた時点で話し始める。

mod clean;
mod readings;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::{Context, bail};
use mozu_audio::Player;
use speechcpp_client::{Client, SpeechRequest, SpeechStream};

pub use readings::Readings;

/// 辞書の内容を解析する。ファイルの配置は呼び出し元で解決する。
pub fn parse_readings(text: &str) -> anyhow::Result<Readings> {
    Readings::from_toml(text).context("読み方の辞書の書き方が違います")
}

/// 読み上げ用に整えた文章。`unknown` は辞書になくて英語のまま残った単語。
#[derive(Debug, Clone, Default)]
pub struct Prepared {
    pub text: String,
    pub unknown: Vec<String>,
}

fn prepare(text: &str, readings: &Readings) -> Prepared {
    let converted = readings.apply(&clean::clean(text));
    Prepared {
        text: converted.text.trim().to_owned(),
        unknown: converted.unknown,
    }
}

#[derive(Debug, Clone)]
pub struct SpeakOptions {
    pub voice: String,
    pub speed: f32,
    pub seed: Option<u64>,
}

#[derive(Clone)]
pub struct Speaker {
    client: Client,
    readings: Readings,
    options: SpeakOptions,
}

impl Speaker {
    pub fn healthy(&self) -> bool {
        self.client.health().is_ok()
    }
    pub fn new(client: Client, readings: Readings, options: SpeakOptions) -> Self {
        Self {
            client,
            readings,
            options,
        }
    }

    /// 読み方の辞書を差し替える。
    pub fn set_readings(&mut self, readings: Readings) {
        self.readings = readings;
    }

    /// 読み上げる文章にする（合成はしない）。
    pub fn prepare(&self, text: &str) -> Prepared {
        prepare(text, &self.readings)
    }

    /// 読み上げる。最後まで鳴らしたら `true`、`stop` で止めたら `false`。
    pub fn speak(&self, text: &str, player: &Player, stop: &AtomicBool) -> anyhow::Result<bool> {
        let started = Instant::now();
        let request = self.request(text)?;
        let mut first = true;
        let complete = self
            .client
            .speech_cancellable(&request, stop, |samples, rate| {
                if first {
                    tracing::debug!("話し始めるまで {} ms", started.elapsed().as_millis());
                    first = false;
                }
                if !stop.load(Ordering::Relaxed) {
                    player.push(samples, rate);
                }
            })?;
        if !complete {
            player.clear();
            return Ok(false);
        }
        Ok(player.wait(stop))
    }

    /// 読み上げた音声を 1 つの WAV にする（再生はしない）。
    pub fn render(&self, text: &str) -> anyhow::Result<Vec<u8>> {
        let mut stream = self.synthesize(text)?;
        let mut samples = Vec::new();
        while let Some(chunk) = stream.next_samples()? {
            samples.extend(chunk);
        }
        mozu_audio::encode_wav(&samples, stream.sample_rate)
    }

    fn synthesize(&self, text: &str) -> anyhow::Result<SpeechStream> {
        Ok(self.client.speech(&self.request(text)?)?)
    }

    fn request(&self, text: &str) -> anyhow::Result<SpeechRequest> {
        let prepared = self.prepare(text);
        if prepared.text.is_empty() {
            bail!("読み上げる文がありません");
        }
        if !prepared.unknown.is_empty() {
            tracing::debug!(
                "辞書にない語（英語のまま読みます）: {}",
                prepared.unknown.join(", ")
            );
        }
        let request = SpeechRequest {
            input: prepared.text,
            voice: self.options.voice.clone(),
            speed: self.options.speed,
            seed: self.options.seed,
        };
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readings_template_is_valid() {
        Readings::from_toml(include_str!("../../../readings.example.toml")).unwrap();
    }

    #[test]
    fn prepare_cleans_and_converts() {
        let readings = Readings::from_toml("github = \"ギットハブ\"").unwrap();
        let prepared = prepare("## **GitHub** と Zebra\n", &readings);
        assert_eq!(prepared.text, "ギットハブ と Zebra");
        assert_eq!(prepared.unknown, ["Zebra"]);
    }
}
