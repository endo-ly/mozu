//! mozu-tray — タスクトレイに常駐する、mozu の入り口。
//!
//! - Ctrl+Win を押している間だけ録音し、文字にして、前面のアプリの入力欄に貼り付ける（音声入力）
//! - Ctrl+Win+Alt で、選択中の文を読み上げる。読み上げ中にもう一度押すと止まる

// リリース版はコンソールを出さない（デバッグ版はログを見るために出す）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod hotkey;
mod insert;
mod reader;
mod sound;
mod tray;
mod ui;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Context;
use mozu_server::{Managed, Spec};
use mozu_speak::{SpeakOptions, Speaker};
use speechcpp_client::Client;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, INFINITE, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::{HSTRING, w};

/// これを立てると、動いている mozu-tray が終わる（`scripts\install.ps1` が使う）。
const QUIT_EVENT: windows::core::PCWSTR = w!(r"Local\mozu-tray-quit");

/// speech.cpp の状態を確かめる間隔。
const HEALTH_INTERVAL: Duration = Duration::from_secs(30);

fn main() {
    if let Err(e) = run() {
        tracing::error!("{e:#}");
        // コンソールがないので、起動に失敗したことはダイアログで知らせる
        let text = HSTRING::from(format!("起動できませんでした。\n\n{e:#}"));
        unsafe { MessageBoxW(None, &text, w!("mozu"), MB_OK | MB_ICONERROR) };
    }
}

fn run() -> anyhow::Result<()> {
    // 2 つ起動すると、1 回の Ctrl+Win で 2 回貼り付けてしまう
    let _instance = unsafe { CreateMutexW(None, true, w!(r"Local\mozu-tray"))? };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        eprintln!("mozu-tray はすでに起動しています");
        return Ok(());
    }

    let logs = mozu_config::data_dir()?.join("logs");
    std::fs::create_dir_all(&logs).with_context(|| format!("{} を作れません", logs.display()))?;
    let _log_guard = init_logging(&logs)?;
    tracing::info!("起動しました ({})", std::env::current_exe()?.display());

    match mozu_config::load_dotenv() {
        Ok(Some(path)) => tracing::info!(".env: {}", path.display()),
        Ok(None) => tracing::info!(".env がないので既定の接続先を使います"),
        Err(e) => tracing::warn!(".env を読めません: {e:#}"),
    }

    let config = mozu_config::load()?;
    let paths = tray::Paths {
        config: mozu_config::config_path()?,
        readings: mozu_config::readings_path()?,
        logs: logs.clone(),
    };
    tracing::info!("設定: {}", paths.config.display());
    for (kind, list) in [
        ("入力", mozu_audio::list_inputs()),
        ("出力", mozu_audio::list_outputs()),
    ] {
        match list {
            Ok(names) => tracing::info!("{kind}デバイス: {}", names.join(" / ")),
            Err(e) => tracing::warn!("{kind}デバイスの一覧を取れません: {e:#}"),
        }
    }
    sound::init(config.sounds);

    let client = Client::new(
        speechcpp_client::base_url_from_env(),
        config.request_timeout(),
    );
    let speaker = Speaker::new(
        client.clone(),
        mozu_config::load_readings()?,
        SpeakOptions {
            voice: config.speech.voice.clone(),
            speed: config.speech.speed,
            seed: None,
            sentence_pause: Duration::from_millis(config.speech.sentence_pause_ms),
            paragraph_pause: Duration::from_millis(config.speech.paragraph_pause_ms),
        },
    );

    let ui = ui::create_window()?;
    let tray = tray::Tray::new(paths)?;

    let (tx, rx) = mpsc::channel();
    let reader = reader::Reader::new(speaker, config.speech.output_device.clone(), ui);
    let app = app::App::new(
        config.dictation,
        client.clone(),
        mozu_config::data_dir()?.join("dictation-last.wav"),
        tx.clone(),
        ui,
        reader,
    );
    let spec = if config.server.managed {
        match Spec::new(&config.server, client.base_url()) {
            Ok(spec) => Some(spec),
            Err(e) => {
                tracing::error!("speech.cpp を起動できません: {e:#}");
                None
            }
        }
    } else {
        None
    };
    spawn_health_watch(client, ui, spec);
    spawn_quit_watch(ui)?;
    std::thread::spawn(move || app.run(rx));

    hotkey::install(tx)?;
    tracing::info!("Ctrl+Win: 音声入力 / Ctrl+Win+Alt: 選択中の文を読み上げ");
    ui::run(ui, tray)?;
    tracing::info!("終了しました");
    Ok(())
}

/// speech.cpp の `/health` を定期的に確かめて、トレイのアイコンに反映する。
///
/// `spec` があるとき（`[server] managed = true`）だけ、応答がなければ speech.cpp を起動する。
/// なければ、つながらないことを灰色のアイコンで知らせるだけにする（サービスは起動しない）。
fn spawn_health_watch(client: speechcpp_client::Client, ui: ui::UiHandle, spec: Option<Spec>) {
    std::thread::spawn(move || {
        let mut managed: Option<Managed> = None;
        let mut last = None;
        loop {
            let mut result = client.health();
            if result.is_err()
                && let Some(spec) = &spec
            {
                if let Some(status) = managed.as_mut().and_then(Managed::exited) {
                    tracing::warn!("speech.cpp が終了しました ({status})。起動し直します");
                    managed = None;
                }
                if managed.is_none() {
                    ui.server(false);
                    match Managed::start(spec) {
                        Ok(started) => {
                            result = wait_until_ready(&client, managed.insert(started));
                        }
                        Err(e) => tracing::error!("{e:#}"),
                    }
                }
            }
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

/// 起動した speech.cpp が応答するまで待つ（モデルの読み込みに数秒かかる）。
/// 初めてモデルを取得するときは長くかかるので、待ちきれなければ次の確認に回す。
fn wait_until_ready(
    client: &speechcpp_client::Client,
    server: &mut Managed,
) -> Result<(), speechcpp_client::Error> {
    const LIMIT: Duration = Duration::from_secs(120);
    let started = Instant::now();
    loop {
        let result = client.health();
        if result.is_ok() || server.exited().is_some() || started.elapsed() > LIMIT {
            return result;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
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

/// コンソールと `%APPDATA%\mozu\logs\tray.<日付>.log` の両方に出す。
/// 既定は info。`RUST_LOG=debug` にすると認識結果や読み上げる文もログに残る。
fn init_logging(
    dir: &std::path::Path,
) -> anyhow::Result<tracing_appender::non_blocking::WorkerGuard> {
    let file = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("tray")
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
