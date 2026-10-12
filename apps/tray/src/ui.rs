//! メインスレッドのメッセージループ。
//!
//! キーボードのフック・タスクトレイ・タイマーは、すべてこのスレッドで動く。
//! ほかのスレッド（認識の処理、speech.cpp の見張り、終了の合図）からは、
//! 見えないウィンドウにメッセージを送って知らせる（`UiHandle`）。

use std::cell::RefCell;

use tray_icon::menu::MenuEvent;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, MSG,
    PBT_APMRESUMEAUTOMATIC, PostMessageW, PostQuitMessage, RegisterClassW, SetTimer,
    TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_DESTROY, WM_POWERBROADCAST, WM_TIMER,
    WM_WTSSESSION_CHANGE, WNDCLASSW, WS_OVERLAPPED, WTS_SESSION_UNLOCK,
};
use windows::core::w;

use crate::tray::{self, Action, Activity, Tray};
use crate::{autostart, hotkey};

const WM_APP_ACTIVITY: u32 = WM_APP + 1;
const WM_APP_SERVER: u32 = WM_APP + 2;
const WM_APP_SPEAKING: u32 = WM_APP + 3;

/// 取りこぼしたキーの解放を補う間隔。
const TIMER_RESYNC: usize = 1;
const RESYNC_MS: u32 = 1_000;
/// フックを入れ直す間隔。
const TIMER_REHOOK: usize = 2;
const REHOOK_MS: u32 = 60_000;

thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

/// メインスレッドの見えないウィンドウ。どのスレッドからでも状態を知らせられる。
#[derive(Debug, Clone, Copy)]
pub struct UiHandle(isize);

impl UiHandle {
    pub fn activity(self, activity: Activity) {
        let code = match activity {
            Activity::Idle => 0,
            Activity::Recording => 1,
            Activity::Busy => 2,
        };
        self.post(WM_APP_ACTIVITY, code);
    }

    pub fn speaking(self, speaking: bool) {
        self.post(WM_APP_SPEAKING, speaking as usize);
    }

    pub fn server(self, ok: bool) {
        self.post(WM_APP_SERVER, ok as usize);
    }

    /// アプリを終える（トレイのアイコンを片付けてから終わる）。
    pub fn close(self) {
        self.post(WM_CLOSE, 0);
    }

    fn post(self, msg: u32, wparam: usize) {
        let _ = unsafe { PostMessageW(Some(self.hwnd()), msg, WPARAM(wparam), LPARAM(0)) };
    }

    fn hwnd(self) -> HWND {
        HWND(self.0 as *mut _)
    }
}

/// 見えないウィンドウを作る。スリープ復帰（WM_POWERBROADCAST）を受け取るため、
/// メッセージ専用ウィンドウ（HWND_MESSAGE）ではなく、表示しない普通のウィンドウにする。
pub fn create_window() -> anyhow::Result<UiHandle> {
    unsafe {
        let instance = GetModuleHandleW(None)?.into();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: w!("mozu-tray"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            anyhow::bail!(
                "ウィンドウクラスを登録できません: {}",
                windows::core::Error::from_thread()
            );
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("mozu-tray"),
            w!("mozu"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )?;
        Ok(UiHandle(hwnd.0 as isize))
    }
}

/// メッセージループを回す。終了の合図（`UiHandle::close` かメニューの「終了」）で戻る。
pub fn run(ui: UiHandle, tray: Tray) -> anyhow::Result<()> {
    TRAY.set(Some(tray));
    unsafe {
        SetTimer(Some(ui.hwnd()), TIMER_RESYNC, RESYNC_MS, None);
        SetTimer(Some(ui.hwnd()), TIMER_REHOOK, REHOOK_MS, None);
        if let Err(e) = WTSRegisterSessionNotification(ui.hwnd(), NOTIFY_FOR_THIS_SESSION) {
            tracing::warn!("画面ロックの解除を受け取れません: {e}");
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
            // メニューのクリックは、トレイのウィンドウの処理（上の Dispatch）の中で届く
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                let action = TRAY.with_borrow(|t| t.as_ref().and_then(|t| t.action_for(&event)));
                match action {
                    Some(Action::SetAutostart(enabled)) => {
                        if let Err(e) = autostart::set(enabled) {
                            tracing::error!("{e:#}");
                        }
                        with_tray(|t| t.sync_autostart());
                        tracing::info!(
                            "ログオン時の起動: {}",
                            if autostart::is_enabled() {
                                "オン"
                            } else {
                                "オフ"
                            }
                        );
                    }
                    Some(Action::Open(path)) => tray::open(&path),
                    Some(Action::Quit) => ui.close(),
                    None => {}
                }
            }
        }
    }
    // ここでアイコンを消す（消さずに終わると、トレイに残骸が残る）
    drop(TRAY.take());
    Ok(())
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_ACTIVITY => {
            let activity = match wparam.0 {
                1 => Activity::Recording,
                2 => Activity::Busy,
                _ => Activity::Idle,
            };
            with_tray(|t| t.set_activity(activity));
        }
        WM_APP_SERVER => with_tray(|t| t.set_server(wparam.0 != 0)),
        WM_APP_SPEAKING => with_tray(|t| t.set_speaking(wparam.0 != 0)),
        WM_TIMER if wparam.0 == TIMER_RESYNC => hotkey::resync(),
        WM_TIMER if wparam.0 == TIMER_REHOOK => hotkey::reinstall(),
        WM_POWERBROADCAST if wparam.0 as u32 == PBT_APMRESUMEAUTOMATIC => {
            tracing::info!("スリープから復帰したので、キーボードのフックを入れ直します");
            hotkey::reinstall();
        }
        WM_WTSSESSION_CHANGE if wparam.0 as u32 == WTS_SESSION_UNLOCK => {
            tracing::info!("画面ロックが解除されたので、キーボードのフックを入れ直します");
            hotkey::reinstall();
        }
        WM_CLOSE => {
            let _ = unsafe { DestroyWindow(hwnd) };
        }
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

/// トレイを借りて操作する。Shell_NotifyIcon の待ち時間に別のメッセージが割り込み、
/// すでに借りている最中だったときは、その更新を飛ばす（次の更新で追いつく）。
fn with_tray(f: impl FnOnce(&mut Tray)) {
    TRAY.with(|t| {
        if let Ok(mut t) = t.try_borrow_mut()
            && let Some(t) = t.as_mut()
        {
            f(t);
        }
    });
}
