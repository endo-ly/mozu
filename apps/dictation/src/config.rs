use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

/// 初回起動時に書き出す設定ファイル。値は `Config::default()` と一致させる（テストで確認）。
const TEMPLATE: &str = r#"# mozu dictation の設定。変更したらアプリを起動し直す。
# speech.cpp の接続先は、ここではなく mozu の .env（MOZU_SPEECHCPP_URL）で指定する

# 認識する言語。"auto" でサーバー側の自動判定
language = "ja"

# 固有名詞や表記のヒント（例: "Riffra, Sonalloy, HyperFrames"）。空なら送らない
prompt = ""

# 入力デバイス名の一部（大文字小文字は区別しない）。空なら Windows の既定の入力デバイス
# 名前の一覧は `mozu-dictation --list-devices` で出る
input_device = ""

# これより短い押し方は誤操作として捨てる（ミリ秒）
min_hold_ms = 300

# 1回の録音の上限（秒）。超えた分は捨てる
max_record_secs = 300

# 文字起こしの応答を待つ上限（秒）
request_timeout_secs = 120

# 貼り付けてから元のクリップボードに戻すまでの待ち時間（ミリ秒）
paste_restore_delay_ms = 300

# 開始・終了・失敗の効果音
sounds = true

# 直前の録音を %APPDATA%\mozu\dictation-last.wav に残す（調べもの用）
keep_last_wav = false
"#;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub language: String,
    pub prompt: String,
    pub input_device: String,
    pub min_hold_ms: u64,
    pub max_record_secs: u32,
    pub request_timeout_secs: u64,
    pub paste_restore_delay_ms: u64,
    pub sounds: bool,
    pub keep_last_wav: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: "ja".into(),
            prompt: String::new(),
            input_device: String::new(),
            min_hold_ms: 300,
            max_record_secs: 300,
            request_timeout_secs: 120,
            paste_restore_delay_ms: 300,
            sounds: true,
            keep_last_wav: false,
        }
    }
}

/// `%APPDATA%\mozu`。設定・ログ・調べもの用の WAV を置く。
pub fn data_dir() -> anyhow::Result<PathBuf> {
    let appdata = std::env::var_os("APPDATA").context("環境変数 APPDATA がありません")?;
    Ok(PathBuf::from(appdata).join("mozu"))
}

/// 設定を読む。ファイルがなければ既定値で書き出してから返す。
pub fn load(path: &Path) -> anyhow::Result<Config> {
    if !path.exists() {
        std::fs::write(path, TEMPLATE)
            .with_context(|| format!("設定ファイルを作れません: {}", path.display()))?;
        return Ok(Config::default());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("設定ファイルを読めません: {}", path.display()))?;
    toml::from_str(&text)
        .with_context(|| format!("設定ファイルの書き方が違います: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_matches_default() {
        let parsed: Config = toml::from_str(TEMPLATE).unwrap();
        assert_eq!(parsed, Config::default());
    }

    #[test]
    fn missing_keys_use_default() {
        let parsed: Config = toml::from_str("language = \"en\"").unwrap();
        assert_eq!(parsed.language, "en");
        assert_eq!(parsed.min_hold_ms, Config::default().min_hold_ms);
    }
}
