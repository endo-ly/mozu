//! mozu dictation — Ctrl+Win を押している間だけ録音し、speech.cpp で文字にして、
//! 前面のアプリの入力欄に貼り付ける。タスクトレイに常駐する。

// リリース版はコンソールを出さない（デバッグ版はログを見るために出す）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod config;
mod hotkey;
mod insert;
mod recorder;
mod sound;
mod tray;
mod ui;

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Context;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, INFINITE, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::{HSTRING, w};

/// これを立てると、動いている mozu-dictation が終わる（`scripts\install.ps1` が使う）。
const QUIT_EVENT: windows::core::PCWSTR = w!(r"Local\mozu-dictation-quit");

/// speech.cpp の状態を確かめる間隔。
const HEALTH_INTERVAL: Duration = Duration::from_secs(30);

fn main() {
    if std::env::args().any(|a| a == "--list-devices") {
        // コンソールのないアプリなので、起動元のターミナルに出す
        let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
        match recorder::list_devices() {
            Ok(names) => names.iter().for_each(|n| println!("{n}")),
            Err(e) => eprintln!("{e:#}"),
        }
        return;
    }

    if let Err(e) = run() {
        tracing::error!("{e:#}");
        // コンソールがないので、起動に失敗したことはダイアログで知らせる
        let text = HSTRING::from(format!("起動できませんでした。\n\n{e:#}"));
        unsafe { MessageBoxW(None, &text, w!("mozu dictation"), MB_OK | MB_ICONERROR) };
    }
}

fn run() -> anyhow::Result<()> {
    // 2 つ起動すると、1 回の Ctrl+Win で 2 回貼り付けてしまう
    let _instance = unsafe { CreateMutexW(None, true, w!(r"Local\mozu-dictation"))? };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        eprintln!("mozu-dictation はすでに起動しています");
        return Ok(());
    }

    let dir = config::data_dir()?;
    let logs = dir.join("logs");
    std::fs::create_dir_all(&logs).with_context(|| format!("{} を作れません", logs.display()))?;
    let _log_guard = init_logging(&logs)?;
    tracing::info!("起動しました ({})", std::env::current_exe()?.display());

    match load_dotenv() {
        Ok(Some(path)) => tracing::info!(".env: {}", path.display()),
        Ok(None) => tracing::info!(".env がないので既定の接続先を使います"),
        Err(e) => tracing::warn!(".env を読めません: {e:#}"),
    }

    let config_path = dir.join("dictation.toml");
    let config = config::load(&config_path)?;
    tracing::info!("設定: {}", config_path.display());
    match recorder::list_devices() {
        Ok(names) => tracing::info!("入力デバイス: {}", names.join(" / ")),
        Err(e) => tracing::warn!("入力デバイスの一覧を取れません: {e:#}"),
    }
    sound::init(config.sounds);

    let ui = ui::create_window()?;
    let tray = tray::Tray::new(&config_path, &logs)?;

    let (tx, rx) = mpsc::channel();
    let app = app::App::new(config, dir.join("dictation-last.wav"), tx.clone(), ui);
    spawn_health_watch(app.client().clone(), ui);
    spawn_quit_watch(ui)?;
    std::thread::spawn(move || app.run(rx));

    hotkey::install(tx)?;
    tracing::info!("Ctrl+Win を押している間だけ録音します");
    ui::run(ui, tray)?;
    tracing::info!("終了しました");
    Ok(())
}

/// speech.cpp の `/health` を定期的に確かめて、トレイのアイコンに反映する。
/// サービスは起動しない（つながらないときは灰色のアイコンで知らせるだけ）。
fn spawn_health_watch(client: speechcpp_client::Client, ui: ui::UiHandle) {
    std::thread::spawn(move || {
        let mut last = None;
        loop {
            let result = client.health();
            let ok = result.is_ok();
            if last != Some(ok) {
                match result {
                    Ok(()) => tracing::info!("speech.cpp に接続しました ({})", client.base_url()),
                    Err(e) => tracing::warn!("{e}"),
                }
                last = Some(ok);
            }
            ui.server(ok);
            std::thread::sleep(HEALTH_INTERVAL);
        }
    });
}

/// 終了の合図（名前付きイベント）を待つ。
fn spawn_quit_watch(ui: ui::UiHandle) -> anyhow::Result<()> {
    let event = unsafe { CreateEventW(None, false, false, QUIT_EVENT)? };
    // HANDLE はスレッドをまたいで送れないので、値として渡す
    let raw = event.0 as isize;
    std::thread::spawn(move || {
        unsafe { WaitForSingleObject(HANDLE(raw as *mut _), INFINITE) };
        tracing::info!("終了の合図を受け取りました");
        ui.close();
    });
    Ok(())
}

/// mozu の `.env` を読む。実行ファイルの場所から上へ、次にカレントディレクトリから上へ探す
/// （`target\release\` から動かしても、インストール先から動かしても見つかるように）。
/// すでに設定されている環境変数は上書きしない。
fn load_dotenv() -> anyhow::Result<Option<PathBuf>> {
    let exe_dir = std::env::current_exe()?.parent().map(Path::to_path_buf);
    let cwd = std::env::current_dir().ok();
    let found = [exe_dir, cwd]
        .into_iter()
        .flatten()
        .flat_map(|dir| dir.ancestors().map(|d| d.join(".env")).collect::<Vec<_>>())
        .find(|p| p.is_file());
    if let Some(path) = &found {
        dotenvy::from_path(path)?;
    }
    Ok(found)
}

/// コンソールと `%APPDATA%\mozu\logs\dictation.<日付>.log` の両方に出す。
/// 既定は info。`RUST_LOG=debug` にすると認識結果の文字列もログに残る。
fn init_logging(
    dir: &std::path::Path,
) -> anyhow::Result<tracing_appender::non_blocking::WorkerGuard> {
    let file = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("dictation")
        .filename_suffix("log")
        .max_log_files(14)
        .build(dir)
        .context("ログファイルを作れません")?;
    let (file, guard) = tracing_appender::non_blocking(file);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_ansi(false)
                .with_writer(file),
        )
        .init();
    Ok(guard)
}
