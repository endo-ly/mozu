//! 文章の読み上げ。下ごしらえ（mozu-text）→ 文ごとの合成（speech.cpp）→ 再生（mozu-audio）。
//!
//! 合成と再生は並行して進める。1 つ目の文を鳴らしている間に次の文を合成するので、
//! 長い文章でも、最初の文ができた時点（約 0.3 秒）で話し始める。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::bail;
use mozu_audio::Player;
use mozu_text::{Pause, Prepared, Readings};
use speechcpp_client::{Client, SpeechRequest};

/// 1 回の合成に渡す最大の文字数（speech.cpp は 1 回 256 トークンまで）。
pub const MAX_CHARS: usize = 100;

/// これより小さい音は無音とみなして、合成した声の前後から切り落とす。
const SILENCE_THRESHOLD: f32 = 0.004;

/// 読点などで長い文を切ったところの間。
const COMMA_PAUSE: Duration = Duration::from_millis(60);

/// 合成を先に進めておく文の数。
const LOOKAHEAD: usize = 2;

#[derive(Debug, Clone)]
pub struct SpeakOptions {
    pub voice: String,
    pub speed: f32,
    pub seed: Option<u64>,
    pub sentence_pause: Duration,
    pub paragraph_pause: Duration,
}

pub struct Speaker {
    client: Client,
    readings: Readings,
    options: SpeakOptions,
}

/// 合成した 1 文。
struct Clip {
    samples: Vec<f32>,
    sample_rate: u32,
    pause: Duration,
}

impl Speaker {
    pub fn new(client: Client, readings: Readings, options: SpeakOptions) -> Self {
        Self {
            client,
            readings,
            options,
        }
    }

    /// 読み上げる文の並びにする（合成はしない）。
    pub fn prepare(&self, text: &str) -> Prepared {
        mozu_text::prepare(text, &self.readings, MAX_CHARS)
    }

    /// 読み上げる。最後まで鳴らしたら `true`、`stop` で止めたら `false`。
    pub fn speak(&self, text: &str, player: &Player, stop: &AtomicBool) -> anyhow::Result<bool> {
        let prepared = self.prepare_checked(text)?;
        let started = Instant::now();
        let (tx, rx) = mpsc::sync_channel::<anyhow::Result<Clip>>(LOOKAHEAD);

        std::thread::scope(|scope| {
            scope.spawn(move || {
                for sentence in &prepared.sentences {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let clip = self.synthesize(&sentence.text, sentence.pause);
                    let failed = clip.is_err();
                    // 受け手が止まって rx を落としていたら、送れずに終わる
                    if tx.send(clip).is_err() || failed {
                        return;
                    }
                }
            });

            let mut first = true;
            for clip in rx {
                if stop.load(Ordering::Relaxed) {
                    player.clear();
                    return Ok(false);
                }
                let clip = clip?;
                if first {
                    tracing::debug!("話し始めるまで {} ms", started.elapsed().as_millis());
                    first = false;
                }
                player.push(&clip.samples, clip.sample_rate);
                player.push_silence(clip.pause);
            }
            Ok(player.wait(stop))
        })
    }

    /// 読み上げた音声を 1 つの WAV にする（再生はしない）。
    pub fn render(&self, text: &str) -> anyhow::Result<Vec<u8>> {
        let prepared = self.prepare_checked(text)?;
        let mut samples = Vec::new();
        let mut rate = None;
        for sentence in &prepared.sentences {
            let clip = self.synthesize(&sentence.text, sentence.pause)?;
            let rate = *rate.get_or_insert(clip.sample_rate);
            samples.extend(mozu_audio::resample(&clip.samples, clip.sample_rate, rate));
            let pause = (clip.pause.as_secs_f64() * rate as f64) as usize;
            samples.extend(std::iter::repeat_n(0.0, pause));
        }
        mozu_audio::wav::encode(&samples, rate.unwrap_or(48_000))
    }

    fn prepare_checked(&self, text: &str) -> anyhow::Result<Prepared> {
        let prepared = self.prepare(text);
        if prepared.sentences.is_empty() {
            bail!("読み上げる文がありません");
        }
        if !prepared.unknown.is_empty() {
            tracing::info!(
                "辞書にない語（英語のまま読みます）: {}",
                prepared.unknown.join(", ")
            );
        }
        Ok(prepared)
    }

    fn synthesize(&self, text: &str, pause: Pause) -> anyhow::Result<Clip> {
        let request = SpeechRequest {
            speed: Some(self.options.speed),
            seed: self.options.seed,
            ..SpeechRequest::wav(text, &self.options.voice)
        };
        let wav = self.client.speech(&request)?;
        let (samples, sample_rate) = mozu_audio::wav::decode(&wav)?;
        let samples = mozu_audio::trim_silence(&samples, SILENCE_THRESHOLD).to_vec();
        let pause = match pause {
            Pause::Comma => COMMA_PAUSE,
            Pause::Sentence => self.options.sentence_pause,
            Pause::Paragraph => self.options.paragraph_pause,
        };
        Ok(Clip {
            samples,
            sample_rate,
            pause,
        })
    }
}
