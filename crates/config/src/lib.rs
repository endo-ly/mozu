//! mozu の設定。`mozu-tray` と `mozu` コマンドで共有する。
//!
//! - `%APPDATA%\mozu\config.toml` — 使い方の好み（声、速さ、マイク、言語など）
//! - `%APPDATA%\mozu\readings.toml` — 読み方の辞書（英単語 → カタカナ）
//!
//! `config.toml` は、なければ説明付きのひな形を書き出す（辞書の読み込みは `mozu-speak`）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, ensure};
use serde::Deserialize;

/// 初回に書き出す設定ファイル。値は `Config::default()` と一致させる（テストで確認）。
const CONFIG_TEMPLATE: &str = r#"# mozu の設定（mozu-tray と mozu コマンドで共有）。変更したら mozu-tray を起動し直す。
# 設定を開くには `mozu config`
# 診断は `mozu doctor`

# speech.cpp の応答を待つ上限（秒）
request_timeout_secs = 120

# 開始・終了・失敗の効果音
sounds = true

[speech]
# 読み上げの声。一覧は `mozu voices`。"none" は声の指定なし。
# 声を足すには、%APPDATA%\mozu\voices\ に参照音声（.wav）か声のファイル（.voice.gguf）を置く（[server] managed のとき）
voice = "none"

# 話す速さ。値と実際の速さは比例しない（1.0 で遅め、1.4 で普通）
speed = 1.4

# 出力デバイス名の一部（大文字小文字は区別しない）。空なら Windows の既定。一覧は `mozu devices`
output_device = ""

[server]
# speech.cpp の接続先
url = "http://127.0.0.1:8012"

# true にすると、mozu-tray が speech.cpp（speech serve）を自分で起動し、終了するときに止める。
# すでに応答があれば起動しない（Windows サービスなどで動かしているなら false のままでよい）。
# `pwsh scripts\setup-speech.ps1 -EnableManaged` で有効にできる
managed = false

# 未使用で自動停止するまでの秒数。1800 = 30 分。0 は自動停止しない（managed のときだけ）
idle_timeout_secs = 1800

# speech.exe の場所。空なら %APPDATA%\mozu\speech\bin\ → PATH の順に探す
exe = ""

# モデルの置き場所（SPEECH_MODEL_DIR）。空なら、mozu が入れた speech.cpp は %APPDATA%\mozu\speech\models、それ以外は speech.cpp の既定
model_dir = ""

# 起動するモデル（合成・認識・検出を 1 つずつ）。初めて使うときに自動で取得される
models = ["irodori-tts-mf", "qwen3-asr-1.7b", "silero-vad"]

# 動かす先。auto（最初の GPU、なければ CPU）/ gpu / cpu / speech devices に出る名前
device = "auto"

# 声の置き場。空なら %APPDATA%\mozu\voices
voices_dir = ""

[dictation]
# 認識する言語。"auto" でサーバー側の自動判定
language = "ja"

# 固有名詞や表記のヒント（例: "OpenAI, GitHub"）。空なら送らない
prompt = ""

# 入力デバイス名の一部（大文字小文字は区別しない）。空なら Windows の既定。一覧は `mozu devices`
input_device = ""

# これより短い押し方は誤操作として捨てる（ミリ秒）
min_hold_ms = 300

# 1回の録音の上限（秒）。超えた分は捨てる
max_record_secs = 300

# 貼り付けてから元のクリップボードに戻すまでの待ち時間（ミリ秒）
paste_restore_delay_ms = 300

# 直前の録音を %APPDATA%\mozu\dictation-last.wav に残す（調べもの用）
keep_last_wav = false
"#;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub request_timeout_secs: u64,
    pub sounds: bool,
    pub speech: SpeechConfig,
    pub server: ServerConfig,
    pub dictation: DictationConfig,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpeechConfig {
    pub voice: String,
    pub speed: f32,
    pub output_device: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub url: String,
    pub managed: bool,
    pub idle_timeout_secs: u64,
    pub exe: String,
    pub model_dir: String,
    pub models: Vec<String>,
    pub device: String,
    pub voices_dir: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DictationConfig {
    pub language: String,
    pub prompt: String,
    pub input_device: String,
    pub min_hold_ms: u64,
    pub max_record_secs: u32,
    pub paste_restore_delay_ms: u64,
    pub keep_last_wav: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            request_timeout_secs: 120,
            sounds: true,
            speech: SpeechConfig::default(),
            server: ServerConfig::default(),
            dictation: DictationConfig::default(),
        }
    }
}

impl Default for SpeechConfig {
    fn default() -> Self {
        Self {
            voice: "none".into(),
            speed: 1.4,
            output_device: String::new(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:8012".into(),
            managed: false,
            idle_timeout_secs: 1800,
            exe: String::new(),
            model_dir: String::new(),
            models: ["irodori-tts-mf", "qwen3-asr-1.7b", "silero-vad"]
                .map(String::from)
                .to_vec(),
            device: "auto".into(),
            voices_dir: String::new(),
        }
    }
}

impl Default for DictationConfig {
    fn default() -> Self {
        Self {
            language: "ja".into(),
            prompt: String::new(),
            input_device: String::new(),
            min_hold_ms: 300,
            max_record_secs: 300,
            paste_restore_delay_ms: 300,
            keep_last_wav: false,
        }
    }
}

impl Config {
    pub fn request_timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_secs)
    }
}

/// `%APPDATA%\mozu`。設定・辞書・ログを置く。
pub fn data_dir() -> anyhow::Result<PathBuf> {
    let appdata = std::env::var_os("APPDATA").context("環境変数 APPDATA がありません")?;
    let dir = PathBuf::from(appdata).join("mozu");
    std::fs::create_dir_all(&dir).with_context(|| format!("{} を作れません", dir.display()))?;
    Ok(dir)
}

pub fn config_path() -> anyhow::Result<PathBuf> {
    Ok(data_dir()?.join("config.toml"))
}

pub fn readings_path() -> anyhow::Result<PathBuf> {
    Ok(data_dir()?.join("readings.toml"))
}

/// 設定を読む。なければひな形を書き出して既定値を返す。
pub fn load() -> anyhow::Result<Config> {
    let path = config_path()?;
    let text = read_or_create(&path, CONFIG_TEMPLATE)?;
    let config: Config = toml::from_str(&text)
        .with_context(|| format!("設定ファイルの書き方が違います: {}", path.display()))?;
    config.validate()?;
    Ok(config)
}

/// ファイルを読む。なければ `template` を書き出し、その中身を返す（あるファイルは書き換えない）。
pub fn read_or_create(path: &Path, template: &str) -> anyhow::Result<String> {
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(template.as_bytes())?;
            Ok(template.to_owned())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => std::fs::read_to_string(path)
            .with_context(|| format!("{} を読めません", path.display())),
        Err(e) => Err(e).with_context(|| format!("{} を作れません", path.display())),
    }
}

/// 設定が壊れていても編集できるよう、解析せずに初期ファイルを用意する。
pub fn ensure_config() -> anyhow::Result<PathBuf> {
    let path = config_path()?;
    read_or_create(&path, CONFIG_TEMPLATE)?;
    Ok(path)
}

impl ServerConfig {
    /// 接続先の検証。値は config.toml だけから読む。
    pub fn base_url(&self) -> anyhow::Result<String> {
        let value = self.url.trim();
        let parsed = url::Url::parse(value).context("server.url が URL ではありません")?;
        ensure!(
            parsed.scheme() == "http" && parsed.host_str().is_some(),
            "接続先は http:// の URL にしてください"
        );
        ensure!(
            parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.query().is_none()
                && parsed.fragment().is_none()
                && parsed.path() == "/",
            "接続先には認証情報・パス・クエリ・フラグメントを指定できません"
        );
        ensure!(parsed.port() != Some(0), "ポート 0 は使えません");
        if self.managed {
            ensure!(
                matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")),
                "managed はループバックの接続先だけで使えます"
            );
            ensure!(
                parsed.port().is_some(),
                "managed の接続先にはポートを指定してください"
            );
        }
        Ok(value.trim_end_matches('/').to_owned())
    }
}

pub fn validate_speed(speed: f32) -> anyhow::Result<()> {
    ensure!(
        speed.is_finite() && (0.25..=4.0).contains(&speed),
        "speech.speed は 0.25〜4.0 の有限の数にしてください"
    );
    Ok(())
}

impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        self.server.base_url()?;
        validate_speed(self.speech.speed)?;
        ensure!(
            (1..=3600).contains(&self.request_timeout_secs),
            "request_timeout_secs は 1〜3600 にしてください"
        );
        ensure!(
            (1..=1800).contains(&self.dictation.max_record_secs),
            "dictation.max_record_secs は 1〜1800 にしてください"
        );
        ensure!(
            self.dictation.min_hold_ms < u64::from(self.dictation.max_record_secs) * 1000,
            "dictation.min_hold_ms は録音上限より短くしてください"
        );
        ensure!(
            self.dictation.paste_restore_delay_ms <= 10000,
            "dictation.paste_restore_delay_ms は 10000 以下にしてください"
        );
        ensure!(
            !self.speech.voice.trim().is_empty(),
            "speech.voice を指定してください（指定なしは none）"
        );
        ensure!(
            !self.dictation.language.trim().is_empty(),
            "dictation.language を指定してください（自動判定は auto）"
        );
        ensure!(
            !self.server.device.trim().is_empty()
                && !self.server.device.chars().any(char::is_control),
            "server.device は auto / gpu / cpu または speech devices の名前にしてください"
        );
        ensure!(
            self.server
                .models
                .iter()
                .all(|m| !m.trim().is_empty() && !m.starts_with('-')),
            "server.models に空文字やオプションは指定できません"
        );
        if self.server.managed {
            ensure!(
                !self.server.models.is_empty(),
                "managed ではモデルを 1 つ以上選んでください"
            );
        }
        Ok(())
    }
}

/// セットアップで明示的に選んだモデルを保存。他の項目とコメントを残す。
pub fn save_models(
    models: &[String],
    managed: bool,
    model_dir: &Path,
    exe: &Path,
    languages: &[String],
) -> anyhow::Result<()> {
    let path = config_path()?;
    let text = read_or_create(&path, CONFIG_TEMPLATE)?;
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .context("設定ファイルを読めません")?;
    let mut values = toml_edit::Array::new();
    for model in models {
        values.push(model.as_str());
    }
    doc["server"]["models"] = toml_edit::value(values);
    doc["server"]["model_dir"] = toml_edit::value(model_dir.to_string_lossy().as_ref());
    doc["server"]["exe"] = toml_edit::value(exe.to_string_lossy().as_ref());
    let current: Config = toml::from_str(&text)?;
    let language = current
        .dictation
        .language
        .split('-')
        .next()
        .unwrap_or("auto");
    if !languages.is_empty() && language != "auto" && !languages.iter().any(|v| v == language) {
        doc["dictation"]["language"] = toml_edit::value("auto");
    }
    if managed {
        doc["server"]["managed"] = toml_edit::value(true);
    }
    let output = doc.to_string();
    let config: Config = toml::from_str(&output)?;
    config.validate()?;
    std::fs::write(path.with_extension("toml.bak"), text)?;
    std::fs::write(&path, output).with_context(|| format!("{} を更新できません", path.display()))
}

/// 辞書の初期作成と読み込み。既存の辞書は変更しない。
pub fn readings_text() -> anyhow::Result<String> {
    read_or_create(
        &readings_path()?,
        include_str!("../../../readings.example.toml"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_matches_default() {
        let parsed: Config = toml::from_str(CONFIG_TEMPLATE).unwrap();
        assert_eq!(parsed, Config::default());
    }

    #[test]
    fn missing_keys_use_default() {
        let parsed: Config = toml::from_str("[speech]\nvoice = \"musa\"").unwrap();
        assert_eq!(parsed.speech.voice, "musa");
        assert_eq!(parsed.speech.speed, SpeechConfig::default().speed);
        assert_eq!(parsed.dictation, DictationConfig::default());
    }
}
