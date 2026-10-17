//! メインスレッドのメッセージループ。
//!
//! キーボードのフック・タスクトレイ・タイマーは、すべてこのスレッドで動く。
//! ほかのスレッド（認識の処理、speech.cpp の見張り、終了の合図）からは、
//! 見えないウィンドウにメッセージを送って知らせる（`UiHandle`）。

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::mpsc;

use tray_icon::menu::MenuEvent;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
};
use windows::Win32::UI::Shell::{
    NIF_INFO, NIF_MESSAGE, NIF_STATE, NIF_TIP, NIIF_INFO, NIIF_RESPECT_QUIET_TIME, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NIN_BALLOONUSERCLICK, NIS_HIDDEN, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, MSG,
    PBT_APMRESUMEAUTOMATIC, PostMessageW, PostQuitMessage, RegisterClassW, SetTimer,
    TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_DESTROY, WM_POWERBROADCAST, WM_TIMER,
    WM_WTSSESSION_CHANGE, WNDCLASSW, WS_OVERLAPPED, WTS_SESSION_UNLOCK,
};
use windows::core::w;

use crate::server_watch::Control;
use crate::tray::{self, Action, Activity, ServerState, Tray};
use crate::{autostart, hotkey};

const WM_APP_ACTIVITY: u32 = WM_APP + 1;
const WM_APP_SERVER: u32 = WM_APP + 2;
const WM_APP_SPEAKING: u32 = WM_APP + 3;
/// 通知（バルーン）を出す合図。システムからのバルーンの出来事も、同じコールバックで届く。
const WM_APP_NOTIFY: u32 = WM_APP + 4;
/// 通知を出すための見えないアイコン（NIS_HIDDEN）。トレイには出ない。
const NOTIFY_ICON_ID: u32 = 1;

/// 通知のタイトルと本文（クリックでログのフォルダを開く）。
const NOTIFY_TITLE: &str = "speech.cpp を起動できません";
const NOTIFY_TEXT: &str =
    "mozu のログを確かめてください。この通知をクリックするとログのフォルダを開きます。";

/// 取りこぼしたキーの解放を補う間隔。
const TIMER_RESYNC: usize = 1;
const RESYNC_MS: u32 = 1_000;
/// フックを入れ直す間隔。
const TIMER_REHOOK: usize = 2;
const REHOOK_MS: u32 = 60_000;

thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
    /// 通知のクリックで開く、ログのフォルダ。
    static LOGS: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// メインスレッドの見えないウィンドウ。どのスレッドからでも状態を知らせられる。
#[derive(Debug, Clone, Copy)]
pub struct UiHandle(isize);

impl UiHandle {
    pub fn activity(self, activity: Activity) {
        self.post(WM_APP_ACTIVITY, activity as usize);
    }

    pub fn speaking(self, speaking: bool) {
        self.post(WM_APP_SPEAKING, speaking as usize);
    }

    pub fn server(self, state: ServerState) {
        self.post(WM_APP_SERVER, state as usize);
    }

    /// 通知（バルーン）を出す。中身は固定（`NOTIFY_TITLE`・`NOTIFY_TEXT`）。
    pub fn notify(self) {
        self.post(WM_APP_NOTIFY, 0);
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
        // 通知（バルーン）を出すための見えないアイコンを登録する
        let icon = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: NOTIFY_ICON_ID,
            uFlags: NIF_MESSAGE | NIF_STATE | NIF_TIP,
            uCallbackMessage: WM_APP_NOTIFY,
            dwState: NIS_HIDDEN,
            dwStateMask: NIS_HIDDEN,
            szTip: utf16::<128>("mozu"),
            ..Default::default()
        };
        if !Shell_NotifyIconW(NIM_ADD, &icon).as_bool() {
            tracing::warn!("通知を使えません: {}", windows::core::Error::from_thread());
        }
        Ok(UiHandle(hwnd.0 as isize))
    }
}

/// メッセージループを回す。終了の合図（`UiHandle::close` かメニューの「終了」）で戻る。
/// `server_ctl` は、メニューからの speech.cpp の停止・起動の合図の送り先。
pub fn run(
    ui: UiHandle,
    tray: Tray,
    logs: PathBuf,
    server_ctl: mpsc::Sender<Control>,
) -> anyhow::Result<()> {
    LOGS.with(|l| *l.borrow_mut() = Some(logs));
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
                    Some(Action::StopServer) => {
                        let _ = server_ctl.send(Control::Stop);
                    }
                    Some(Action::StartServer) => {
                        let _ = server_ctl.send(Control::Start);
                    }
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
        WM_APP_SERVER => with_tray(|t| {
            t.set_server(match wparam.0 {
                1 => ServerState::Up,
                2 => ServerState::Down,
                3 => ServerState::Stopped,
                _ => ServerState::Checking,
            });
        }),
        WM_APP_SPEAKING => with_tray(|t| t.set_speaking(wparam.0 != 0)),
        // 通知を出す合図（lparam == 0）と、システムからのバルーンの出来事（lparam == NIN_*）が来る
        WM_APP_NOTIFY => {
            if lparam.0 == 0 {
                show_balloon(hwnd);
            } else if lparam.0 == NIN_BALLOONUSERCLICK as isize {
                LOGS.with(|l| {
                    if let Some(dir) = l.borrow().as_ref() {
                        tray::open(dir);
                    }
                });
            }
        }
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
        WM_DESTROY => {
            // 見えないアイコンも片付ける
            let icon = NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: NOTIFY_ICON_ID,
                ..Default::default()
            };
            let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &icon) };
            unsafe { PostQuitMessage(0) }
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

/// 固定の文字列を、NOTIFYICONDATA の固定長バッファ用にヌル終端で並べる。
fn utf16<const N: usize>(text: &str) -> [u16; N] {
    let mut buf = [0; N];
    for (i, ch) in text.encode_utf16().take(N - 1).enumerate() {
        buf[i] = ch;
    }
    buf
}

/// 通知（バルーン）を出す。内容は固定。Windows 10 以降はトースト通知になる。
fn show_balloon(hwnd: HWND) {
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: NOTIFY_ICON_ID,
        uFlags: NIF_INFO,
        szInfo: utf16::<256>(NOTIFY_TEXT),
        szInfoTitle: utf16::<64>(NOTIFY_TITLE),
        dwInfoFlags: NIIF_INFO | NIIF_RESPECT_QUIET_TIME,
        ..Default::default()
    };
    if !unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
        tracing::warn!("通知を出せません: {}", windows::core::Error::from_thread());
    }
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
