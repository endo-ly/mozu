//! タスクトレイのアイコンとメニュー。アイコンの色で今の状態を見せる。
//!
//! 白: 待機中 / 赤: 録音中 / 黄: 認識中 / 青: 読み上げ中 / 灰: speech.cpp につながらない

use std::path::{Path, PathBuf};

use anyhow::Context;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

use crate::autostart;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Recording,
    Busy,
}

/// メニューで選ばれた操作。
///
/// ShellExecute などはメッセージを処理することがあるので、トレイを借りたまま実行せず、
/// 呼び出し側で借用を終えてから `perform` する。
pub enum Action {
    /// チェックが切り替わった後の状態。
    SetAutostart(bool),
    Open(PathBuf),
    Quit,
}

/// メニューから開くファイルとフォルダ。
pub struct Paths {
    pub config: PathBuf,
    pub readings: PathBuf,
    pub logs: PathBuf,
}

pub struct Tray {
    icon: TrayIcon,
    status: MenuItem,
    autostart: CheckMenuItem,
    open_config: MenuItem,
    open_readings: MenuItem,
    open_logs: MenuItem,
    quit: MenuItem,
    paths: Paths,
    activity: Activity,
    speaking: bool,
    /// `None` はまだ確かめていない。
    server: Option<bool>,
}

impl Tray {
    pub fn new(paths: Paths) -> anyhow::Result<Self> {
        let status = MenuItem::new("", false, None);
        let autostart = CheckMenuItem::new("ログオン時に起動", true, autostart::is_enabled(), None);
        let open_config = MenuItem::new("設定ファイルを開く", true, None);
        let open_readings = MenuItem::new("読み方の辞書を開く", true, None);
        let open_logs = MenuItem::new("ログのフォルダを開く", true, None);
        let quit = MenuItem::new("終了", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &status,
            &PredefinedMenuItem::separator(),
            &autostart,
            &open_config,
            &open_readings,
            &open_logs,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon(Activity::Idle, false, None)?)
            .build()
            .context("タスクトレイにアイコンを出せません")?;

        let tray = Self {
            icon,
            status,
            autostart,
            open_config,
            open_readings,
            open_logs,
            quit,
            paths,
            activity: Activity::Idle,
            speaking: false,
            server: None,
        };
        tray.refresh();
        Ok(tray)
    }

    pub fn set_activity(&mut self, activity: Activity) {
        if self.activity != activity {
            self.activity = activity;
            self.refresh();
        }
    }

    pub fn set_speaking(&mut self, speaking: bool) {
        if self.speaking != speaking {
            self.speaking = speaking;
            self.refresh();
        }
    }

    pub fn set_server(&mut self, ok: bool) {
        if self.server != Some(ok) {
            self.server = Some(ok);
            self.refresh();
        }
    }

    pub fn action_for(&self, event: &MenuEvent) -> Option<Action> {
        let id = event.id();
        if id == self.autostart.id() {
            // クリックした時点で、チェックはもう切り替わっている
            Some(Action::SetAutostart(self.autostart.is_checked()))
        } else if id == self.open_config.id() {
            Some(Action::Open(self.paths.config.clone()))
        } else if id == self.open_readings.id() {
            Some(Action::Open(self.paths.readings.clone()))
        } else if id == self.open_logs.id() {
            Some(Action::Open(self.paths.logs.clone()))
        } else if id == self.quit.id() {
            Some(Action::Quit)
        } else {
            None
        }
    }

    /// 実際の登録状態をチェックに反映する。
    pub fn sync_autostart(&self) {
        self.autostart.set_checked(autostart::is_enabled());
    }

    fn refresh(&self) {
        let activity = match self.activity {
            Activity::Idle if self.speaking => "読み上げ中",
            Activity::Idle => "待機中",
            Activity::Recording => "録音中",
            Activity::Busy => "認識中",
        };
        let server = match self.server {
            None => "speech.cpp を確認中",
            Some(true) => "speech.cpp 接続中",
            Some(false) => "speech.cpp につながりません",
        };
        let text = format!("{activity}（{server}）");
        self.status.set_text(&text);
        let _ = self.icon.set_tooltip(Some(format!("mozu — {text}")));
        match icon(self.activity, self.speaking, self.server) {
            Ok(icon) => {
                let _ = self.icon.set_icon(Some(icon));
            }
            Err(e) => tracing::warn!("アイコンを作れません: {e}"),
        }
    }
}

pub fn open(path: &Path) {
    // .toml には関連付けがないことが多く、ShellExecute は「アプリを選ぶ」ダイアログを
    // 出して成功を返してしまうので、ファイルはメモ帳で直接開く
    if path.is_file() {
        if let Err(e) = std::process::Command::new("notepad.exe").arg(path).spawn() {
            tracing::warn!("{} をメモ帳で開けません: {e}", path.display());
        }
        return;
    }
    let path = HSTRING::from(path.as_os_str());
    let result = unsafe { ShellExecuteW(None, w!("open"), &path, None, None, SW_SHOWNORMAL) };
    // 32 以下はエラー
    if result.0 as usize <= 32 {
        tracing::warn!("{} を開けません", path);
    }
}

/// 32×32 の丸いアイコンを描く。
fn icon(activity: Activity, speaking: bool, server: Option<bool>) -> anyhow::Result<Icon> {
    const SIZE: u32 = 32;
    let fill: [u8; 3] = match (activity, server) {
        (Activity::Recording, _) => [0xE5, 0x39, 0x35],
        (Activity::Busy, _) => [0xFF, 0xB3, 0x00],
        (Activity::Idle, Some(false)) => [0x8A, 0x8A, 0x8A],
        (Activity::Idle, _) if speaking => [0x42, 0xA5, 0xF5],
        (Activity::Idle, _) => [0xF2, 0xF2, 0xF2],
    };
    let outline = [0x20, 0x20, 0x20];
    let center = (SIZE as f32 - 1.0) / 2.0;
    let radius = SIZE as f32 / 2.0 - 1.5;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let d = ((x as f32 - center).powi(2) + (y as f32 - center).powi(2)).sqrt();
            // 縁を 1px なめらかにする
            let alpha = (radius + 0.5 - d).clamp(0.0, 1.0);
            let color = if d > radius - 2.5 { outline } else { fill };
            rgba.extend_from_slice(&color);
            rgba.push((alpha * 255.0) as u8);
        }
    }
    Ok(Icon::from_rgba(rgba, SIZE, SIZE)?)
}
