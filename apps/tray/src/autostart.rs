//! ログオン時の自動起動。ユーザーのスタートアップ
//! （`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`）に、今動いている実行ファイルを登録する。

use anyhow::Context;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{HSTRING, PCWSTR, w};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE_NAME: PCWSTR = w!("mozu-tray");

/// 登録するコマンド（今動いている実行ファイルのパス）。
fn command() -> anyhow::Result<String> {
    let exe = std::env::current_exe().context("実行ファイルの場所が分かりません")?;
    Ok(format!("\"{}\"", exe.display()))
}

/// 今動いている実行ファイルが登録されているか。別の場所の実行ファイルが登録されていれば false。
pub fn is_enabled() -> bool {
    let Ok(expected) = command() else {
        return false;
    };
    let mut buf = [0u16; 1024];
    let mut size = std::mem::size_of_val(&buf) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE_NAME,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return false;
    }
    let len = (size as usize / 2).saturating_sub(1); // 末尾の NUL を除く
    String::from_utf16_lossy(&buf[..len]).eq_ignore_ascii_case(&expected)
}

pub fn set(enabled: bool) -> anyhow::Result<()> {
    if enabled {
        let command = HSTRING::from(command()?);
        let bytes = (command.len() + 1) * 2; // NUL を含む
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE_NAME,
                REG_SZ.0,
                Some(command.as_ptr().cast()),
                bytes as u32,
            )
        }
        .ok()
        .context("スタートアップに登録できません")?;
    } else {
        let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE_NAME) };
        if status != ERROR_FILE_NOT_FOUND {
            status.ok().context("スタートアップから外せません")?;
        }
    }
    Ok(())
}
