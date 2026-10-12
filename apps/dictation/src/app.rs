//! 録音から貼り付けまでの状態の管理。
//!
//! 状態は `Idle → Recording → Busy（認識と貼り付け）→ Idle`。
//! Busy の間にホットキーが押されても無視して、二重に録音・送信しない。

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use speechcpp_client::{Client, TranscribeOptions};

use crate::config::Config;
use crate::insert::{self, Target};
use crate::recorder::{Recorder, Recording};
use crate::sound::{self, Cue};
use crate::tray::Activity;
use crate::ui::UiHandle;

pub enum Event {
    HotkeyPressed,
    HotkeyReleased,
    HotkeyCancelled,
    /// 認識と貼り付けが終わった（成否はログと効果音で知らせ済み）。
    Done,
}

enum State {
    Idle,
    Recording {
        recorder: Recorder,
        started: Instant,
        target: Target,
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
    config: Config,
    client: Client,
    options: TranscribeOptions,
    last_wav: Option<PathBuf>,
    events: Sender<Event>,
    ui: UiHandle,
}

impl App {
    pub fn new(config: Config, last_wav: PathBuf, events: Sender<Event>, ui: UiHandle) -> Self {
        let client = Client::new(
            speechcpp_client::base_url_from_env(),
            Duration::from_secs(config.request_timeout_secs),
        );
        let non_empty = |s: &str| (!s.is_empty()).then(|| s.to_owned());
        let options = TranscribeOptions {
            language: non_empty(&config.language),
            prompt: non_empty(&config.prompt),
            model: None,
        };
        let last_wav = config.keep_last_wav.then_some(last_wav);
        Self {
            config,
            client,
            options,
            last_wav,
            events,
            ui,
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn run(self, events: Receiver<Event>) {
        let mut state = State::Idle;
        for event in events {
            let before = state.activity();
            state = self.step(state, event);
            if state.activity() != before {
                self.ui.activity(state.activity());
            }
        }
    }

    fn step(&self, state: State, event: Event) -> State {
        match (state, event) {
            (State::Idle, Event::HotkeyPressed) => {
                let target = Target::foreground();
                match Recorder::start(&self.config.input_device, self.config.max_record_secs) {
                    Ok(recorder) => {
                        sound::play(Cue::Start);
                        tracing::info!("録音開始");
                        State::Recording {
                            recorder,
                            started: Instant::now(),
                            target,
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
                    recorder,
                    started,
                    target,
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
                self.spawn_transcription(recording, target);
                State::Busy
            }
            (State::Recording { recorder, .. }, Event::HotkeyCancelled) => {
                drop(recorder.stop());
                tracing::info!("ほかのキーが押されたので録音を取り消しました");
                State::Idle
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
    fn spawn_transcription(&self, recording: Recording, target: Target) {
        let client = self.client.clone();
        let options = self.options.clone();
        let last_wav = self.last_wav.clone();
        let restore_delay = Duration::from_millis(self.config.paste_restore_delay_ms);
        let events = self.events.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<()> {
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
