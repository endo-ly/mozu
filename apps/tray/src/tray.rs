//! タスクトレイのアイコンとメニュー。鳥の絵に付く丸で今の状態を見せる。
//!
//! 丸なし: 待機中 / 赤: 録音中 / 黄: 認識中 / 青: 読み上げ中 / 鳥が灰色: speech.cpp につながらない（手動で止めているときも灰色）

use std::path::{Path, PathBuf};

use anyhow::Context;
use tray_icon::menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
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

/// speech.cpp との接続の状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerState {
    /// まだ確かめていない。
    Checking,
    /// つながっている。
    Up,
    /// つながらない。
    Down,
    /// managed の speech.cpp を、トレイのメニューで止めている。
    Stopped,
}

/// メニューで選ばれた操作。
///
/// ShellExecute などはメッセージを処理することがあるので、トレイを借りたまま実行せず、
/// 呼び出し側で借用を終えてから `perform` する。
pub enum Action {
    /// チェックが切り替わった後の状態。
    SetAutostart(bool),
    Open(PathBuf),
    /// managed の speech.cpp を止める。
    StopServer,
    /// 手動で止めた speech.cpp を起動する。
    StartServer,
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
    /// managed のときだけある。「speech.cpp を止める／起動する」
    server_toggle: Option<MenuItem>,
    autostart: CheckMenuItem,
    open_config: MenuItem,
    open_readings: MenuItem,
    open_logs: MenuItem,
    quit: MenuItem,
    paths: Paths,
    activity: Activity,
    speaking: bool,
    server: ServerState,
}

impl Tray {
    pub fn new(paths: Paths, managed: bool) -> anyhow::Result<Self> {
        let status = MenuItem::new("", false, None);
        let server_toggle = managed.then(|| MenuItem::new("speech.cpp を止める", false, None));
        let autostart = CheckMenuItem::new("ログオン時に起動", true, autostart::is_enabled(), None);
        let open_config = MenuItem::new("設定ファイルを開く", true, None);
        let open_readings = MenuItem::new("読み方の辞書を開く", true, None);
        let open_logs = MenuItem::new("ログのフォルダを開く", true, None);
        let quit = MenuItem::new("終了", true, None);
        let menu = Menu::new();
        let top = PredefinedMenuItem::separator();
        let bottom = PredefinedMenuItem::separator();
        let mut items: Vec<&dyn IsMenuItem> = vec![&status, &top];
        if let Some(item) = &server_toggle {
            items.push(item);
        }
        items.push(&autostart);
        items.push(&open_config);
        items.push(&open_readings);
        items.push(&open_logs);
        items.push(&bottom);
        items.push(&quit);
        menu.append_items(&items)?;

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon(Activity::Idle, false, None)?)
            .build()
            .context("タスクトレイにアイコンを出せません")?;

        let tray = Self {
            icon,
            status,
            server_toggle,
            autostart,
            open_config,
            open_readings,
            open_logs,
            quit,
            paths,
            activity: Activity::Idle,
            speaking: false,
            server: ServerState::Checking,
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

    pub fn set_server(&mut self, state: ServerState) {
        if self.server != state {
            self.server = state;
            match (&self.server_toggle, state) {
                (Some(item), ServerState::Checking) => item.set_enabled(false),
                (Some(item), ServerState::Stopped) => {
                    item.set_text("speech.cpp を起動する");
                    item.set_enabled(true);
                }
                (Some(item), _) => {
                    item.set_text("speech.cpp を止める");
                    item.set_enabled(true);
                }
                (None, _) => {}
            }
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
        } else if self
            .server_toggle
            .as_ref()
            .is_some_and(|item| id == item.id())
        {
            // 止まっているときは「起動する」、それ以外は「止める」
            Some(if self.server == ServerState::Stopped {
                Action::StartServer
            } else {
                Action::StopServer
            })
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
            ServerState::Checking => "speech.cpp を確認中",
            ServerState::Up => "speech.cpp 接続中",
            ServerState::Down => "speech.cpp につながらない",
            ServerState::Stopped => "speech.cpp を止めています",
        };
        let text = format!("{activity}（{server}）");
        self.status.set_text(&text);
        let _ = self.icon.set_tooltip(Some(format!("mozu — {text}")));
        let reachable = match self.server {
            ServerState::Up => Some(true),
            ServerState::Checking => None,
            ServerState::Down | ServerState::Stopped => Some(false),
        };
        match icon(self.activity, self.speaking, reachable) {
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

/// トレイに出す大きさ。Windows が画面の拡大率に合わせて縮める。
const SIZE: usize = 64;

/// 鳥の絵（256×256 の RGBA）。最初に使うときに読み込む。
fn bird() -> &'static [u8] {
    static BIRD: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    BIRD.get_or_init(|| {
        let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!("../assets/icon.png")));
        let mut reader = decoder.read_info().expect("icon.png を読めません");
        let mut buf = vec![
            0;
            reader
                .output_buffer_size()
                .expect("icon.png が大きすぎます")
        ];
        let info = reader.next_frame(&mut buf).expect("icon.png を読めません");
        assert_eq!(
            (info.width, info.height),
            (256, 256),
            "icon.png は 256×256 にする"
        );
        assert_eq!(
            info.color_type,
            png::ColorType::Rgba,
            "icon.png は RGBA にする"
        );
        buf
    })
}

fn average(sum: [u32; 3], alpha: u32) -> Option<[u8; 3]> {
    Some([
        sum[0].checked_div(alpha)? as u8,
        sum[1].checked_div(alpha)? as u8,
        sum[2].checked_div(alpha)? as u8,
    ])
}

/// 鳥のアイコンを作る。つながらないときは灰色に、録音・認識・読み上げ中は右下に色の丸を付ける。
fn icon(activity: Activity, speaking: bool, server: Option<bool>) -> anyhow::Result<Icon> {
    let src = bird();
    let scale = 256 / SIZE;
    let offline = server == Some(false) && activity == Activity::Idle;
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    // 縮める。透明な画素が色を濁らせないよう、不透明度で重みを付けて平均する
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (mut sum, mut alpha) = ([0u32; 3], 0u32);
            for dy in 0..scale {
                for dx in 0..scale {
                    let p = ((y * scale + dy) * 256 + x * scale + dx) * 4;
                    let a = u32::from(src[p + 3]);
                    for c in 0..3 {
                        sum[c] += u32::from(src[p + c]) * a;
                    }
                    alpha += a;
                }
            }
            let out = (y * SIZE + x) * 4;
            // 完全に透明な画素（alpha == 0）は色を付けない
            if let Some(color) = average(sum, alpha) {
                let mut color = color;
                if offline {
                    let gray =
                        (u32::from(color[0]) * 3 + u32::from(color[1]) * 6 + u32::from(color[2]))
                            / 10;
                    color = [(gray / 2) as u8 + 0x50; 3];
                }
                rgba[out..out + 3].copy_from_slice(&color);
            }
            rgba[out + 3] = (alpha / (scale * scale) as u32) as u8;
        }
    }

    let badge: Option<[u8; 3]> = match (activity, speaking) {
        (Activity::Recording, _) => Some([0xE5, 0x39, 0x35]),
        (Activity::Busy, _) => Some([0xFF, 0xB3, 0x00]),
        (Activity::Idle, true) => Some([0x42, 0xA5, 0xF5]),
        (Activity::Idle, false) => None,
    };
    if let Some(fill) = badge {
        let radius = SIZE as f32 * 0.24;
        let (cx, cy) = (SIZE as f32 - radius - 1.0, SIZE as f32 - radius - 1.0);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                let cover = (radius + 0.5 - d).clamp(0.0, 1.0);
                if cover > 0.0 {
                    let color = if d > radius - 3.0 { [0xFF; 3] } else { fill };
                    let out = (y * SIZE + x) * 4;
                    for c in 0..3 {
                        let old = f32::from(rgba[out + c]);
                        rgba[out + c] = (old + (f32::from(color[c]) - old) * cover) as u8;
                    }
                    rgba[out + 3] = rgba[out + 3].max((cover * 255.0) as u8);
                }
            }
        }
    }
    Ok(Icon::from_rgba(rgba, SIZE as u32, SIZE as u32)?)
}
