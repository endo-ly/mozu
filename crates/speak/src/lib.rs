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
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use mozu_audio::Player;
use speechcpp_client::{Client, SpeechRequest, SpeechStream};

pub use readings::Readings;

/// 初回に %APPDATA%\mozu\readings.toml へコピーする、配布用の読み方の辞書。
const READINGS_TEMPLATE: &str = include_str!("../../../readings.example.toml");

/// 読み方の辞書（`%APPDATA%\mozu\readings.toml`）を読む。なければ、配布用の例をコピーして作る。
pub fn load_readings() -> anyhow::Result<Readings> {
    let path = mozu_config::readings_path()?;
    let text = mozu_config::read_or_create(&path, READINGS_TEMPLATE)?;
    Readings::from_toml(&text)
        .with_context(|| format!("読み方の辞書の書き方が違います: {}", path.display()))
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
        let mut stream = self.synthesize(text)?;
        let sample_rate = stream.sample_rate;
        let (tx, rx) = mpsc::channel::<anyhow::Result<Vec<f32>>>();

        std::thread::scope(|scope| {
            // 届くのを待つ間も止められるよう、受け取りは別のスレッドで行う。
            // 受け手が止まって rx を落とすと送れずに終わり、stream が落ちて接続が切れる
            //（サーバーは残りの合成をやめる）
            scope.spawn(move || {
                loop {
                    match stream.next_samples() {
                        Ok(Some(samples)) => {
                            if tx.send(Ok(samples)).is_err() {
                                return;
                            }
                        }
                        Ok(None) => return,
                        Err(e) => {
                            let _ = tx.send(Err(e.into()));
                            return;
                        }
                    }
                }
            });

            // 止めたとき、受け取りのスレッドを待つ前に rx を落とすため、この中に移す
            let rx = rx;
            let mut first = true;
            loop {
                if stop.load(Ordering::Relaxed) {
                    player.clear();
                    return Ok(false);
                }
                match rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(samples) => {
                        let samples = samples?;
                        if first {
                            tracing::debug!("話し始めるまで {} ms", started.elapsed().as_millis());
                            first = false;
                        }
                        player.push(&samples, sample_rate);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            Ok(player.wait(stop))
        })
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
        let prepared = self.prepare(text);
        if prepared.text.is_empty() {
            bail!("読み上げる文がありません");
        }
        if !prepared.unknown.is_empty() {
            tracing::info!(
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
        let stream = self.client.speech(&request)?;
        if let Some(seed) = stream.seed {
            tracing::debug!("合成の種: {seed}（--seed で同じ読み方になる）");
        }
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readings_template_is_valid() {
        Readings::from_toml(READINGS_TEMPLATE).unwrap();
    }

    #[test]
    fn prepare_cleans_and_converts() {
        let readings = Readings::from_toml("github = \"ギットハブ\"").unwrap();
        let prepared = prepare("## **GitHub** と Zebra\n", &readings);
        assert_eq!(prepared.text, "ギットハブ と Zebra");
        assert_eq!(prepared.unknown, ["Zebra"]);
    }
}
