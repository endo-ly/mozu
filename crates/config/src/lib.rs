//! mozu の設定。`mozu-tray` と `mozu` コマンドで共有する。
//!
//! - `%APPDATA%\mozu\config.toml` — 使い方の好み（声、速さ、マイク、言語など）
//! - `%APPDATA%\mozu\readings.toml` — 読み方の辞書（英単語 → カタカナ）
//! - mozu の `.env` — 環境ごとの値（speech.cpp の接続先 `MOZU_SPEECHCPP_URL`）
//!
//! どちらのファイルも、なければ説明付きのひな形を書き出す。

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use mozu_text::Readings;
use serde::Deserialize;

/// 初回に書き出す設定ファイル。値は `Config::default()` と一致させる（テストで確認）。
const CONFIG_TEMPLATE: &str = r#"# mozu の設定（mozu-tray と mozu コマンドで共有）。変更したら mozu-tray を起動し直す。
# speech.cpp の接続先は、ここではなく mozu の .env（MOZU_SPEECHCPP_URL）で指定する
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

# 文と文の間、段落と段落の間（ミリ秒）
sentence_pause_ms = 200
paragraph_pause_ms = 450

[server]
# true にすると、mozu-tray が speech.cpp（speech serve）を自分で起動し、終了するときに止める。
# すでに応答があれば起動しない（Windows サービスなどで動かしているなら false のままでよい）。
# `pwsh scripts\setup-speech.ps1` で speech.cpp を入れると true になる
managed = false

# speech.exe の場所。空なら %APPDATA%\mozu\speech\bin\ → PATH の順に探す
exe = ""

# モデルの置き場所（SPEECH_MODEL_DIR）。空なら、mozu が入れた speech.cpp は %APPDATA%\mozu\speech\models、それ以外は speech.cpp の既定
model_dir = ""

# 起動するモデル（合成・認識・検出を 1 つずつ）。初めて使うときに自動で取得される
models = ["irodori-tts-mf", "qwen3-asr-1.7b", "silero-vad"]

# 動かす先。auto（最初の GPU、なければ CPU）/ gpu / cpu
device = "auto"

# 声の置き場。空なら %APPDATA%\mozu\voices
voices_dir = ""

[dictation]
# 認識する言語。"auto" でサーバー側の自動判定
language = "ja"

# 固有名詞や表記のヒント（例: "Riffra, Sonalloy, HyperFrames"）。空なら送らない
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

/// 初回に %APPDATA%\mozu\readings.toml へコピーする、配布用の読み方の辞書（リポジトリ直下の readings.example.toml）。
const READINGS_TEMPLATE: &str = include_str!("../../../readings.example.toml");

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
    pub sentence_pause_ms: u64,
    pub paragraph_pause_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub managed: bool,
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
            sentence_pause_ms: 200,
            paragraph_pause_ms: 450,
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            managed: false,
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
    toml::from_str(&text)
        .with_context(|| format!("設定ファイルの書き方が違います: {}", path.display()))
}

/// 読み方の辞書を読む。なければ、配布用の例（`readings.example.toml`）をコピーして作る。
pub fn load_readings() -> anyhow::Result<Readings> {
    let path = readings_path()?;
    let text = read_or_create(&path, READINGS_TEMPLATE)?;
    Readings::from_toml(&text)
        .with_context(|| format!("読み方の辞書の書き方が違います: {}", path.display()))
}

fn read_or_create(path: &Path, template: &str) -> anyhow::Result<String> {
    if !path.exists() {
        std::fs::write(path, template)
            .with_context(|| format!("{} を作れません", path.display()))?;
        return Ok(template.to_owned());
    }
    std::fs::read_to_string(path).with_context(|| format!("{} を読めません", path.display()))
}

/// mozu の `.env` を読む。実行ファイルの場所から上へ、次にカレントディレクトリから上へ探す
/// （`target\release\` からでも `bin\` からでも見つかるように）。
/// すでに設定されている環境変数は上書きしない。見つけた `.env` のパスを返す。
pub fn load_dotenv() -> anyhow::Result<Option<PathBuf>> {
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

    #[test]
    fn readings_template_is_valid() {
        Readings::from_toml(READINGS_TEMPLATE).unwrap();
    }
}
