//! 選択中の文の読み上げ（Ctrl+Win+Alt）。読み上げ中にもう一度押すと止まる。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use anyhow::Context;
use mozu_audio::Player;
use mozu_speak::Speaker;

use crate::insert;
use crate::sound::{self, Cue};
use crate::ui::UiHandle;

pub struct Reader {
    speaker: Speaker,
    output_device: String,
    ui: UiHandle,
    running: Option<Running>,
}

struct Running {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Reader {
    pub fn new(speaker: Speaker, output_device: String, ui: UiHandle) -> Self {
        Self {
            speaker,
            output_device,
            ui,
            running: None,
        }
    }

    /// 読み上げ中なら止め、そうでなければ選択中の文を読み上げる。
    pub fn toggle(&mut self) {
        if let Some(running) = self.running.take_if(|r| !r.thread.is_finished()) {
            running.stop.store(true, Ordering::Relaxed);
            tracing::info!("読み上げを止めました");
            return;
        }

        let stop = Arc::new(AtomicBool::new(false));
        // 辞書を足したらすぐ効くよう、読み上げるたびに読み直す（読めなければ前の辞書のまま）
        let mut speaker = self.speaker.clone();
        match mozu_config::load_readings() {
            Ok(readings) => speaker.set_readings(readings),
            Err(e) => tracing::warn!("{e:#}。前の辞書で読み上げます"),
        }
        let output_device = self.output_device.clone();
        let ui = self.ui;
        let stop_flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            ui.speaking(true);
            if let Err(e) = read_selection(&speaker, &output_device, &stop_flag) {
                sound::play(Cue::Error);
                tracing::error!("{e:#}");
            }
            ui.speaking(false);
        });
        self.running = Some(Running { stop, thread });
    }
}

fn read_selection(speaker: &Speaker, output_device: &str, stop: &AtomicBool) -> anyhow::Result<()> {
    let Some(text) = insert::copy_selection()? else {
        anyhow::bail!("選択されている文がありません");
    };
    tracing::info!("読み上げ開始 ({} 文字)", text.chars().count());
    tracing::debug!("読み上げる文: {text}");
    let player = Player::open(output_device).context("スピーカーを開けません")?;
    if speaker.speak(&text, &player, stop)? {
        tracing::info!("読み上げ終了");
    }
    Ok(())
}
