//! Ctrl+Win の押しっぱなしを検出する低レベルキーボードフック。
//!
//! `RegisterHotKey` ではキーを離した瞬間が分からないので、`WH_KEYBOARD_LL` で
//! すべてのキーを見て、Ctrl と Win が両方押されている間を 1 回の録音とする。
//! キーは横取りせず、そのまま流す。フックとそのメッセージループは、メインスレッド（`ui`）で動く。

use std::cell::{Cell, RefCell};
use std::sync::OnceLock;
use std::sync::mpsc::Sender;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_LCONTROL, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RWIN,
    VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::app::Event;

/// 割り当てのない仮想キー。Win を離したときにスタートメニューが開かないよう、
/// 押している途中に 1 回送って「Win と別のキーの組み合わせだった」ことにする。
const VK_MASK: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);

static EVENTS: OnceLock<Sender<Event>> = OnceLock::new();

thread_local! {
    static CHORD: RefCell<Chord> = RefCell::new(Chord::default());
    static HOOK: Cell<Option<HHOOK>> = const { Cell::new(None) };
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    /// 録音中。
    Active,
    /// ほかのキーが混ざった。Ctrl と Win を両方離すまで何もしない。
    Blocked,
}

#[derive(Debug, Default)]
struct Chord {
    lctrl: bool,
    rctrl: bool,
    lwin: bool,
    rwin: bool,
    phase: Phase,
}

#[derive(Debug, PartialEq, Eq)]
enum Output {
    Pressed,
    Released,
    Cancelled,
}

impl Chord {
    fn ctrl(&self) -> bool {
        self.lctrl || self.rctrl
    }

    fn win(&self) -> bool {
        self.lwin || self.rwin
    }

    /// キー 1 回分の変化を受けて、状態を進める。
    /// `other_modifier_down` は Shift / Alt が押されているか（Ctrl+Shift+Win などを除くため）。
    fn on_key(&mut self, vk: VIRTUAL_KEY, down: bool, other_modifier_down: bool) -> Option<Output> {
        let is_chord_key = match vk {
            VK_LCONTROL => {
                self.lctrl = down;
                true
            }
            VK_RCONTROL => {
                self.rctrl = down;
                true
            }
            VK_LWIN => {
                self.lwin = down;
                true
            }
            VK_RWIN => {
                self.rwin = down;
                true
            }
            _ => false,
        };

        let both = self.ctrl() && self.win();
        let neither = !self.ctrl() && !self.win();

        match self.phase {
            Phase::Idle if down && !is_chord_key && (self.ctrl() || self.win()) => {
                // Ctrl+C や Win+E の途中。続けて Ctrl+Win になっても録音しない
                self.phase = Phase::Blocked;
                None
            }
            Phase::Idle if both && !other_modifier_down => {
                self.phase = Phase::Active;
                Some(Output::Pressed)
            }
            Phase::Active if down && !is_chord_key => {
                // Ctrl+Win+→ などの Windows のショートカットとして使われた
                self.phase = Phase::Blocked;
                Some(Output::Cancelled)
            }
            Phase::Active if !both => {
                self.phase = if neither { Phase::Idle } else { Phase::Blocked };
                Some(Output::Released)
            }
            Phase::Blocked if neither => {
                self.phase = Phase::Idle;
                None
            }
            _ => None,
        }
    }
}

/// 4 つのキーの実際の状態（`GetAsyncKeyState`）。
#[derive(Debug, Default, Clone, Copy)]
struct Physical {
    lctrl: bool,
    rctrl: bool,
    lwin: bool,
    rwin: bool,
}

impl Physical {
    fn read() -> Self {
        let down = |vk: VIRTUAL_KEY| unsafe { GetAsyncKeyState(vk.0.into()) } < 0;
        Self {
            lctrl: down(VK_LCONTROL),
            rctrl: down(VK_RCONTROL),
            lwin: down(VK_LWIN),
            rwin: down(VK_RWIN),
        }
    }
}

impl Chord {
    /// フックが取りこぼしたキーの解放を、実際のキーの状態で補う。
    fn resync(&mut self, physical: Physical) -> Option<Output> {
        self.lctrl = physical.lctrl;
        self.rctrl = physical.rctrl;
        self.lwin = physical.lwin;
        self.rwin = physical.rwin;
        let neither = !self.ctrl() && !self.win();
        match self.phase {
            Phase::Active if !(self.ctrl() && self.win()) => {
                self.phase = if neither { Phase::Idle } else { Phase::Blocked };
                Some(Output::Released)
            }
            Phase::Blocked if neither => {
                self.phase = Phase::Idle;
                None
            }
            _ => None,
        }
    }
}

/// フックを入れる。メッセージループは呼び出し側（`ui`）が同じスレッドで回す。
pub fn install(events: Sender<Event>) -> anyhow::Result<()> {
    EVENTS
        .set(events)
        .map_err(|_| anyhow::anyhow!("フックはすでに動いています"))?;
    set_hook()
}

/// フックを入れ直す。Windows は、フックの応答が遅れたとき（スリープ復帰の直後など）に
/// 知らせずにフックを外すことがあるので、定期的とスリープ復帰・ロック解除のときに呼ぶ。
/// 録音中は、キーを離した瞬間を取りこぼさないよう入れ直さない。
pub fn reinstall() {
    if CHORD.with_borrow(|c| c.phase) == Phase::Active {
        return;
    }
    if let Some(old) = HOOK.take() {
        let _ = unsafe { UnhookWindowsHookEx(old) };
    }
    if let Err(e) = set_hook() {
        tracing::error!("キーボードのフックを入れ直せません: {e}");
    }
}

/// 実際のキーの状態と突き合わせる。フックが外れていて Ctrl+Win の解放を
/// 取りこぼしても、録音が止まらないままにならないよう、1 秒ごとに呼ぶ。
pub fn resync() {
    let output = CHORD.with_borrow_mut(|c| c.resync(Physical::read()));
    if let Some(output) = output {
        tracing::warn!("キーの解放を取りこぼしていたので、録音を終えます");
        emit(output);
    }
}

fn set_hook() -> anyhow::Result<()> {
    unsafe {
        let module = GetModuleHandleW(None)?;
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), Some(module.into()), 0)?;
        HOOK.set(Some(hook));
    }
    Ok(())
}

fn emit(output: Output) {
    if let Some(tx) = EVENTS.get() {
        let _ = tx.send(match output {
            Output::Pressed => Event::HotkeyPressed,
            Output::Released => Event::HotkeyReleased,
            Output::Cancelled => Event::HotkeyCancelled,
        });
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: HC_ACTION のとき lparam は KBDLLHOOKSTRUCT を指す
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // 自分で送った Ctrl+V や VK_MASK は数えない
        if (info.flags & LLKHF_INJECTED).0 == 0 {
            let down = match wparam.0 as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
                WM_KEYUP | WM_SYSKEYUP => Some(false),
                _ => None,
            };
            if let Some(down) = down {
                let vk = VIRTUAL_KEY(info.vkCode as u16);
                let output = CHORD.with_borrow_mut(|c| c.on_key(vk, down, other_modifier_down()));
                if let Some(output) = output {
                    if output == Output::Pressed {
                        mask_start_menu();
                    }
                    emit(output);
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn other_modifier_down() -> bool {
    // フックの中でも、今処理中のキーより前の状態は GetAsyncKeyState に反映されている
    unsafe { GetAsyncKeyState(VK_SHIFT.0.into()) < 0 || GetAsyncKeyState(VK_MENU.0.into()) < 0 }
}

fn mask_start_menu() {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_MASK,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    unsafe {
        SendInput(&inputs, size_of::<INPUT>() as i32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_C, VK_RIGHT};

    fn feed(chord: &mut Chord, keys: &[(VIRTUAL_KEY, bool)]) -> Vec<Output> {
        keys.iter()
            .filter_map(|&(vk, down)| chord.on_key(vk, down, false))
            .collect()
    }

    #[test]
    fn press_and_release_in_any_order() {
        let mut c = Chord::default();
        let out = feed(
            &mut c,
            &[
                (VK_LWIN, true),
                (VK_LCONTROL, true),
                (VK_LWIN, false),
                (VK_LCONTROL, false),
            ],
        );
        assert_eq!(out, [Output::Pressed, Output::Released]);
        assert_eq!(c.phase, Phase::Idle);
    }

    #[test]
    fn key_repeat_does_not_retrigger() {
        let mut c = Chord::default();
        let out = feed(
            &mut c,
            &[
                (VK_LCONTROL, true),
                (VK_LWIN, true),
                (VK_LCONTROL, true),
                (VK_LWIN, true),
            ],
        );
        assert_eq!(out, [Output::Pressed]);
    }

    #[test]
    fn re_press_after_partial_release_waits_for_full_release() {
        let mut c = Chord::default();
        let out = feed(
            &mut c,
            &[
                (VK_LCONTROL, true),
                (VK_LWIN, true),
                (VK_LWIN, false),
                (VK_LWIN, true), // Ctrl を押したまま Win を押し直しても始まらない
                (VK_LWIN, false),
                (VK_LCONTROL, false),
                (VK_RCONTROL, true),
                (VK_RWIN, true),
            ],
        );
        assert_eq!(out, [Output::Pressed, Output::Released, Output::Pressed]);
    }

    #[test]
    fn windows_shortcut_cancels() {
        let mut c = Chord::default();
        let out = feed(
            &mut c,
            &[(VK_LCONTROL, true), (VK_LWIN, true), (VK_RIGHT, true)],
        );
        assert_eq!(out, [Output::Pressed, Output::Cancelled]);
        let out = feed(
            &mut c,
            &[(VK_RIGHT, false), (VK_LWIN, false), (VK_LCONTROL, false)],
        );
        assert!(out.is_empty());
        assert_eq!(c.phase, Phase::Idle);
    }

    #[test]
    fn other_combo_in_progress_blocks() {
        let mut c = Chord::default();
        let out = feed(
            &mut c,
            &[(VK_LCONTROL, true), (VK_C, true), (VK_LWIN, true)],
        );
        assert!(out.is_empty());
    }

    #[test]
    fn resync_releases_a_stuck_recording() {
        let mut c = Chord::default();
        feed(&mut c, &[(VK_LCONTROL, true), (VK_LWIN, true)]);
        // 解放のイベントが届かなかった
        assert_eq!(c.resync(Physical::default()), Some(Output::Released));
        assert_eq!(c.phase, Phase::Idle);
        assert_eq!(c.resync(Physical::default()), None);
    }

    #[test]
    fn resync_keeps_a_held_chord() {
        let mut c = Chord::default();
        feed(&mut c, &[(VK_LCONTROL, true), (VK_LWIN, true)]);
        let held = Physical {
            lctrl: true,
            lwin: true,
            ..Default::default()
        };
        assert_eq!(c.resync(held), None);
        assert_eq!(c.phase, Phase::Active);
    }

    #[test]
    fn shift_held_does_not_start() {
        let mut c = Chord::default();
        assert_eq!(c.on_key(VK_LCONTROL, true, true), None);
        assert_eq!(c.on_key(VK_LWIN, true, true), None);
    }
}
