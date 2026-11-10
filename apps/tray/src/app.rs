//! ホットキーを受けて、録音から貼り付けまで（音声入力）と、読み上げを動かす。
//!
//! 音声入力の状態は `Idle → Recording → Busy（認識と貼り付け）→ Idle`。
//! Busy の間にホットキーが押されても無視して、二重に録音・送信しない。
//! 読み上げ（`Reader`）は音声入力とは別に動く。

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use mozu_audio::{Recorder, Recording};
use mozu_config::DictationConfig;
use speechcpp_client::{Client, TranscribeOptions};

use crate::insert::{self, Target};
use crate::reader::Reader;
use crate::sound::{self, Cue};
use crate::tray::Activity;
use crate::ui::UiHandle;

/// 録音開始の音を鳴らすまでの待ち。Ctrl+Win に続けて Alt を押した（読み上げの合図）ときに、
/// 録音開始の音が一瞬鳴ってしまわないようにする。
const START_CUE_DELAY: Duration = Duration::from_millis(150);

pub enum Event {
    HotkeyPressed,
    HotkeyReleased,
    HotkeyCancelled,
    /// Ctrl+Win+Alt。
    ReadPressed,
    /// 認識と貼り付けが終わった（成否はログと効果音で知らせ済み）。
    Done,
}

enum State {
    Idle,
    Recording {
        usage: Option<mozu_server::UsageGuard>,
        recorder: Recorder,
        started: Instant,
        target: Target,
        cue_played: bool,
    },
    Busy,
}

impl State {
    fn activity(&self) -> Activity {
        match self {
            State::Idle => Activity::Idle,
            State::Recording { .. } => Activity::Recording,
            State::Busy => Activity::Busy,
        }
    }
}

pub struct App {
    config: DictationConfig,
    client: Client,
    options: TranscribeOptions,
    last_wav: Option<PathBuf>,
    events: Sender<Event>,
    ui: UiHandle,
    reader: Reader,
    usage: Option<mozu_server::UsageSession>,
}

impl App {
    pub fn new(
        config: DictationConfig,
        client: Client,
        last_wav: PathBuf,
        events: Sender<Event>,
        ui: UiHandle,
        reader: Reader,
    ) -> Self {
        let options = TranscribeOptions {
            language: Some(config.language.clone()),
            prompt: Some(config.prompt.clone()),
        };
        let last_wav = config.keep_last_wav.then_some(last_wav);
        Self {
            config,
            client,
            options,
            last_wav,
            events,
            ui,
            reader,
            usage: None,
        }
    }

    pub fn with_usage(mut self, usage: Option<mozu_server::UsageSession>) -> Self {
        self.usage = usage;
        self
    }

    pub fn run(mut self, events: Receiver<Event>) {
        let mut state = State::Idle;
        loop {
            // 録音開始の音がまだなら、その時刻までだけ待つ
            let cue_due = match &state {
                State::Recording {
                    started,
                    cue_played: false,
                    ..
                } => Some(*started + START_CUE_DELAY),
                _ => None,
            };
            let event = match cue_due {
                Some(due) => {
                    match events.recv_timeout(due.saturating_duration_since(Instant::now())) {
                        Ok(event) => event,
                        Err(RecvTimeoutError::Timeout) => {
                            if let State::Recording { cue_played, .. } = &mut state {
                                sound::play(Cue::Start);
                                *cue_played = true;
                            }
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                None => match events.recv() {
                    Ok(event) => event,
                    Err(_) => return,
                },
            };

            let before = state.activity();
            state = self.step(state, event);
            if state.activity() != before {
                self.ui.activity(state.activity());
            }
        }
    }

    fn step(&mut self, state: State, event: Event) -> State {
        match (state, event) {
            (State::Idle, Event::HotkeyPressed) => {
                let usage = match self.usage.as_ref().map(|usage| usage.begin()).transpose() {
                    Ok(usage) => usage,
                    Err(e) => {
                        sound::play(Cue::Error);
                        tracing::error!("{e:#}");
                        return State::Idle;
                    }
                };
                let target = Target::foreground();
                match Recorder::start(&self.config.input_device, self.config.max_record_secs) {
                    Ok(recorder) => {
                        tracing::info!("録音開始");
                        State::Recording {
                            usage,
                            recorder,
                            started: Instant::now(),
                            target,
                            cue_played: false,
                        }
                    }
                    Err(e) => {
                        sound::play(Cue::Error);
                        tracing::error!("録音を開始できません: {e:#}");
                        State::Idle
                    }
                }
            }
            (
                State::Recording {
                    usage,
                    recorder,
                    started,
                    target,
                    ..
                },
                Event::HotkeyReleased,
            ) => {
                let recording = recorder.stop();
                let held = started.elapsed();
                if held < Duration::from_millis(self.config.min_hold_ms) {
                    tracing::info!("短すぎるので捨てました ({} ms)", held.as_millis());
                    return State::Idle;
                }
                sound::play(Cue::Stop);
                tracing::info!("録音終了 ({:.1} 秒)", recording.seconds());
                if recording.truncated {
                    tracing::warn!(
                        "録音が上限 {} 秒を超えたので、超えた分は捨てました",
                        self.config.max_record_secs
                    );
                }
                self.spawn_transcription(recording, target, usage);
                State::Busy
            }
            (State::Recording { recorder, .. }, Event::HotkeyCancelled) => {
                drop(recorder.stop());
                tracing::info!("ほかのキーが押されたので録音を取り消しました");
                State::Idle
            }
            (state, Event::ReadPressed) => {
                let state = match state {
                    State::Recording { recorder, .. } => {
                        drop(recorder.stop());
                        State::Idle
                    }
                    other => other,
                };
                self.reader.toggle();
                state
            }
            (State::Busy, Event::Done) => State::Idle,
            (State::Busy, Event::HotkeyPressed) => {
                tracing::info!("認識中なので無視しました");
                State::Busy
            }
            (state, _) => state,
        }
    }

    /// 認識と貼り付けは別スレッドで行い、終わったら `Event::Done` を送る。
    fn spawn_transcription(
        &self,
        recording: Recording,
        target: Target,
        usage_guard: Option<mozu_server::UsageGuard>,
    ) {
        let usage = self.usage.clone();
        let client = self.client.clone();
        let options = self.options.clone();
        let last_wav = self.last_wav.clone();
        let restore_delay = Duration::from_millis(self.config.paste_restore_delay_ms);
        let events = self.events.clone();
        std::thread::spawn(move || {
            let _usage_guard = usage_guard;
            let result = (|| -> anyhow::Result<()> {
                if let Some(usage) = usage {
                    usage.wait_ready(&std::sync::atomic::AtomicBool::new(false), || {
                        client.health().is_ok()
                    })?;
                }
                let wav = recording.to_wav()?;
                drop(recording);
                if let Some(path) = &last_wav
                    && let Err(e) = std::fs::write(path, &wav)
                {
                    tracing::warn!("{} に保存できません: {e}", path.display());
                }

                let started = Instant::now();
                let text = client.transcribe(&wav, &options)?;
                let text = text.trim();
                tracing::info!(
                    "認識しました ({} 文字, {} ms)",
                    text.chars().count(),
                    started.elapsed().as_millis()
                );
                tracing::debug!("認識結果: {text}");
                if text.is_empty() {
                    anyhow::bail!("何も認識されませんでした");
                }

                insert::insert(text, target, restore_delay)
            })();
            if let Err(e) = result {
                sound::play(Cue::Error);
                tracing::error!("{e:#}");
            }
            let _ = events.send(Event::Done);
        });
    }
}
