//! 読み上げる前の文章の下ごしらえ。
//!
//! 1. `clean` — Markdown の記号・URL・コードの塊を取り除く
//! 2. `Readings::apply` — 英単語をカタカナにする（Irodori-TTS は英語の読みが崩れるため）
//! 3. `split` — 1 回の合成に収まる長さに、文の切れ目で区切る

mod clean;
mod readings;
mod split;

pub use clean::clean;
pub use readings::{Converted, Readings};
pub use split::{Pause, Sentence, split};

/// 読み上げ用に整えた文の並び。`unknown` は辞書になくて英語のまま残った単語。
#[derive(Debug, Clone, Default)]
pub struct Prepared {
    pub sentences: Vec<Sentence>,
    pub unknown: Vec<String>,
}

/// `clean` → `Readings::apply` → `split` をまとめて行う。
pub fn prepare(text: &str, readings: &Readings, max_chars: usize) -> Prepared {
    let converted = readings.apply(&clean(text));
    Prepared {
        sentences: split(&converted.text, max_chars),
        unknown: converted.unknown,
    }
}
