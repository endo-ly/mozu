//! 読み上げる前の文章の下ごしらえ。
//!
//! 1. `clean` — Markdown の記号・URL・コードの塊を取り除く
//! 2. `Readings::apply` — 英単語をカタカナにする（Irodori-TTS は英語の読みが崩れるため）
//!
//! 文の区切りは speech.cpp に任せる（文ごとに合成し、文と文の間を空けてつなぐ）。

mod clean;
mod readings;

pub use clean::clean;
pub use readings::{Converted, Readings};

/// 読み上げ用に整えた文章。`unknown` は辞書になくて英語のまま残った単語。
#[derive(Debug, Clone, Default)]
pub struct Prepared {
    pub text: String,
    pub unknown: Vec<String>,
}

/// `clean` → `Readings::apply` をまとめて行う。
pub fn prepare(text: &str, readings: &Readings) -> Prepared {
    let converted = readings.apply(&clean(text));
    Prepared {
        text: converted.text.trim().to_owned(),
        unknown: converted.unknown,
    }
}
