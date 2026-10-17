//! `mozu doctor` — 動かすのに必要なものがそろっているかを確かめ、足りないものの直し方を示す。

use std::process::{Command, Stdio};

use mozu_config::Config;
use speechcpp_client::Client;

enum Level {
    Ok,
    /// なくても動くが、一部の機能が使えない。
    Warn,
    Fail,
}

struct Check {
    level: Level,
    label: &'static str,
    detail: String,
    /// 直し方。
    hint: Option<String>,
}

impl Check {
    fn ok(label: &'static str, detail: impl Into<String>) -> Self {
        Self {
            level: Level::Ok,
            label,
            detail: detail.into(),
            hint: None,
        }
    }

    fn warn(label: &'static str, detail: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            level: Level::Warn,
            label,
            detail: detail.into(),
            hint: Some(hint.into()),
        }
    }

    fn fail(label: &'static str, detail: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            level: Level::Fail,
            label,
            detail: detail.into(),
            hint: Some(hint.into()),
        }
    }
}

/// 確かめた結果を表示する。直すべきものがあれば `false`。
pub fn run() -> bool {
    let mut checks = Vec::new();

    let config = match mozu_config::load() {
        Ok(config) => {
            let path = mozu_config::config_path().map(|p| p.display().to_string());
            checks.push(Check::ok("設定", path.unwrap_or_default()));
            config
        }
        Err(e) => {
            checks.push(Check::fail(
                "設定",
                format!("{e:#}"),
                "設定ファイルを直すか、消して作り直させてください",
            ));
            Config::default()
        }
    };

    let client = Client::from_env(config.request_timeout());
    let server_up = client.health().is_ok();
    checks.extend(check_server(&config, &client, server_up));
    if server_up {
        checks.extend(check_models(&config, &client));
    }
    checks.push(check_device(
        "マイク",
        mozu_audio::list_inputs(),
        "Windows の設定でマイクを有効にし、`mozu devices` で名前を確かめる",
    ));
    checks.push(check_device(
        "スピーカー",
        mozu_audio::list_outputs(),
        "Windows の設定で出力デバイスを確かめる",
    ));
    checks.push(check_ffmpeg());

    print(&checks)
}

fn print(checks: &[Check]) -> bool {
    let mut healthy = true;
    for check in checks {
        let mark = match check.level {
            Level::Ok => "[OK]  ",
            Level::Warn => "[注意]",
            Level::Fail => {
                healthy = false;
                "[NG]  "
            }
        };
        println!("{mark} {}: {}", check.label, check.detail);
        if let Some(hint) = &check.hint {
            println!("       → {hint}");
        }
    }
    println!();
    if healthy {
        println!("問題ありません");
    } else {
        println!("[NG] の項目を直してください");
    }
    healthy
}

fn check_server(config: &Config, client: &Client, server_up: bool) -> Vec<Check> {
    let exe = mozu_server::find_exe(&config.server.exe);
    let mut checks = Vec::new();

    if server_up {
        checks.push(Check::ok(
            "speech.cpp のサーバー",
            format!("応答あり ({})", client.base_url()),
        ));
    } else {
        let detail = format!("応答がありません ({})", client.base_url());
        checks.push(match (&exe, config.server.managed) {
            (None, _) => Check::fail(
                "speech.cpp のサーバー",
                detail,
                "speech.cpp が見つかりません。mozu のフォルダで `pwsh scripts\\setup-speech.ps1` を実行してください",
            ),
            (Some(_), true) => Check::fail(
                "speech.cpp のサーバー",
                detail,
                "mozu-tray が起動します。mozu-tray が動いているか、ログ（%APPDATA%\\mozu\\logs\\speech.log）を確かめてください",
            ),
            (Some(_), false) => Check::fail(
                "speech.cpp のサーバー",
                detail,
                "設定の [server] managed を true にして mozu-tray を起動し直すか、自分で `speech serve` を起動してください",
            ),
        });
    }

    checks.push(match &exe {
        Some(path) => {
            let version = mozu_server::version(path).unwrap_or_else(|e| format!("({e:#})"));
            Check::ok("speech.exe", format!("{} — {version}", path.display()))
        }
        // サーバーが別の方法で動いていれば、speech.exe はなくても困らない
        None if server_up => Check::ok(
            "speech.exe",
            "mozu からは使っていません（別の方法で動いています）",
        ),
        None => Check::warn(
            "speech.exe",
            "見つかりません",
            "`pwsh scripts\\setup-speech.ps1` で入れられます",
        ),
    });
    if config.server.managed && !server_up {
        match mozu_server::Spec::new(&config.server, client.base_url()) {
            Ok(spec) => checks.push(Check::ok("起動のしかた", spec.command_line())),
            Err(e) => checks.push(Check::fail(
                "起動のしかた",
                format!("{e:#}"),
                "設定の [server] と .env の接続先を確かめてください",
            )),
        }
    }
    checks
}

/// 合成・認識のモデルがあるか、設定の声が使えるか。
fn check_models(config: &Config, client: &Client) -> Vec<Check> {
    let models = match client.models() {
        Ok(models) => models,
        Err(e) => {
            return vec![Check::fail(
                "モデル",
                format!("一覧を取れません: {e}"),
                "speech.cpp のバージョンを確かめてください",
            )];
        }
    };
    let mut checks = Vec::new();
    for (task, label, example) in [
        ("recognition", "認識モデル", "qwen3-asr-1.7b"),
        ("synthesis", "合成モデル", "irodori-tts-mf"),
    ] {
        let names: Vec<_> = models
            .iter()
            .filter(|m| m.task == task)
            .map(|m| m.id.as_str())
            .collect();
        checks.push(if names.is_empty() {
            Check::fail(
                label,
                "載っていません",
                format!("`speech serve` に {example} を足してください（設定の [server] models）"),
            )
        } else {
            Check::ok(label, names.join(", "))
        });
    }

    let voices: Vec<_> = models
        .iter()
        .filter(|m| m.task == "synthesis")
        .flat_map(|m| m.voices.iter().map(|v| v.name.as_str()))
        .collect();
    let voice = config.speech.voice.as_str();
    if !voices.is_empty() {
        checks.push(if voices.contains(&voice) {
            Check::ok("声", format!("{voice}（使える声: {}）", voices.join(", ")))
        } else {
            Check::fail(
                "声",
                format!("設定の声「{voice}」がありません（使える声: {}）", voices.join(", ")),
                "設定の speech.voice を直すか、声のファイルを %APPDATA%\\mozu\\voices\\ に置いて speech.cpp を起動し直してください",
            )
        });
    }
    checks
}

fn check_device(label: &'static str, list: anyhow::Result<Vec<String>>, hint: &str) -> Check {
    match list {
        Ok(names) if !names.is_empty() => {
            // 既定のものがあればそれを、なければ最初のものを見せる
            let shown = names
                .iter()
                .find(|n| n.contains("(既定)"))
                .unwrap_or(&names[0]);
            Check::ok(label, shown.clone())
        }
        Ok(_) => Check::fail(label, "見つかりません", hint),
        Err(e) => Check::fail(label, format!("{e:#}"), hint),
    }
}

fn check_ffmpeg() -> Check {
    let found = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if found {
        Check::ok("FFmpeg", "見つかりました")
    } else {
        Check::warn(
            "FFmpeg",
            "見つかりません",
            "`mozu transcribe` で WAV 以外のファイルを使うときに必要です（`winget install Gyan.FFmpeg`）",
        )
    }
}
