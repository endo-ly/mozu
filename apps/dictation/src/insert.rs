//! 認識結果を、前面のアプリの入力欄に入れる。
//!
//! クリップボードに入れて Ctrl+V を送り、少し待ってから元のテキストに戻す。
//! 貼り付けた文字はクリップボードの履歴やクラウド同期に残さない。

use std::time::{Duration, Instant};

use anyhow::Context;
use arboard::{Clipboard, SetExtWindows};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LWIN,
    VK_MENU, VK_RWIN, VK_SHIFT, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

/// 前面のウィンドウ。録音開始時に覚えておき、貼り付ける前に変わっていないか確かめる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target(isize);

// HWND は値として比べるだけで、ほかのスレッドから操作はしない。
unsafe impl Send for Target {}

impl Target {
    pub fn foreground() -> Self {
        Self(unsafe { GetForegroundWindow() }.0 as isize)
    }

    fn hwnd(self) -> HWND {
        HWND(self.0 as *mut _)
    }
}

pub fn insert(text: &str, target: Target, restore_delay: Duration) -> anyhow::Result<()> {
    let mut clipboard = Clipboard::new().context("クリップボードを開けません")?;

    // Ctrl や Win が押されたままだと Ctrl+Win+V などになってしまう
    wait_modifiers_released(Duration::from_secs(3));

    if Target::foreground() != target {
        // 手で貼り付けられるよう、こちらは履歴にも残す
        clipboard
            .set_text(text)
            .context("クリップボードに書けません")?;
        tracing::debug!(target = ?target.hwnd(), "前面のウィンドウが変わった");
        anyhow::bail!(
            "前面のウィンドウが変わったので貼り付けませんでした（結果はクリップボードにあります）"
        );
    }

    // テキスト以外（画像など）は戻せない。その場合は認識結果が残る
    let previous = clipboard.get_text().ok();

    clipboard
        .set()
        .exclude_from_monitoring()
        .text(text)
        .context("クリップボードに書けません")?;
    send_paste();

    // 貼り付け先のアプリがクリップボードを読み終えるのを待つ
    std::thread::sleep(restore_delay);
    if let Some(previous) = previous {
        clipboard
            .set()
            .exclude_from_monitoring()
            .text(previous)
            .context("元のクリップボードを戻せません")?;
    }
    Ok(())
}

fn wait_modifiers_released(timeout: Duration) {
    let start = Instant::now();
    let modifiers = [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN];
    while start.elapsed() < timeout {
        let any_down = modifiers
            .iter()
            .any(|vk| unsafe { GetAsyncKeyState(vk.0.into()) } < 0);
        if !any_down {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    tracing::warn!("修飾キーが離されないまま貼り付けます");
}

fn send_paste() {
    let key = |vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                // スキャンコードで読むアプリ（ゲームエンジン製の UI など）のために付ける
                wScan: unsafe { MapVirtualKeyW(vk.0.into(), MAPVK_VK_TO_VSC) } as u16,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let none = KEYBD_EVENT_FLAGS(0);
    let inputs = [
        key(VK_CONTROL, none),
        key(VK_V, none),
        key(VK_V, KEYEVENTF_KEYUP),
        key(VK_CONTROL, KEYEVENTF_KEYUP),
    ];
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        // 管理者権限のウィンドウ（UIPI）が相手だと、ここで弾かれる
        tracing::warn!("Ctrl+V を送れませんでした（{sent}/{} 件）", inputs.len());
    }
}
