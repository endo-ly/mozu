//! 英単語をカタカナに置き換える。
//!
//! 1. 複数語の項目（"pull request" など）を先に置き換える
//! 2. 英数字のかたまりを丸ごと辞書で引く（"Node.js"、"mp3"）
//! 3. なければ、記号・数字・大文字の切れ目で分けて引く（"PowerShell" → Power + Shell）
//! 4. それでもなければ、すべて大文字の 6 文字以下は 1 文字ずつ読む（"GPU" → ジーピーユー）
//! 5. 残った語は英語のまま残し、`unknown` で知らせる（辞書に足す候補）

use std::collections::HashMap;

/// 大文字だけの語を 1 文字ずつ読むときの上限。
const ACRONYM_MAX: usize = 6;

const LETTERS: [&str; 26] = [
    "エー",
    "ビー",
    "シー",
    "ディー",
    "イー",
    "エフ",
    "ジー",
    "エイチ",
    "アイ",
    "ジェー",
    "ケー",
    "エル",
    "エム",
    "エヌ",
    "オー",
    "ピー",
    "キュー",
    "アール",
    "エス",
    "ティー",
    "ユー",
    "ブイ",
    "ダブリュー",
    "エックス",
    "ワイ",
    "ゼット",
];

#[derive(Debug, Clone)]
pub struct Readings {
    /// 小文字にした 1 語の見出し → 読み。
    words: HashMap<String, String>,
    /// 小文字にした複数語の見出し → 読み。長いものから順に並べる。
    phrases: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Converted {
    pub text: String,
    /// 辞書になく、英語のまま残った語（重複なし、出てきた順）。
    pub unknown: Vec<String>,
}

impl Readings {
    /// 辞書（`見出し = "読み"` の TOML）から作る。
    pub(crate) fn from_toml(toml_text: &str) -> Result<Self, toml::de::Error> {
        let table: HashMap<String, String> = toml::from_str(toml_text)?;
        let mut words = HashMap::new();
        let mut phrases = Vec::new();
        for (key, value) in table {
            let key = key.trim().to_ascii_lowercase();
            if key.contains(' ') {
                phrases.push((key, value));
            } else {
                words.insert(key, value);
            }
        }
        phrases.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
        Ok(Self { words, phrases })
    }

    pub(crate) fn apply(&self, text: &str) -> Converted {
        let text = self.replace_phrases(text);
        let mut out = String::with_capacity(text.len());
        let mut unknown: Vec<String> = Vec::new();
        let mut rest = text.as_str();
        while let Some(start) = rest.find(|c: char| c.is_ascii_alphanumeric()) {
            out.push_str(&rest[..start]);
            rest = &rest[start..];
            let len = word_len(rest);
            let word = &rest[..len];
            rest = &rest[len..];
            match self.read_word(word) {
                Ok(reading) => out.push_str(&reading),
                Err(partial) => {
                    out.push_str(&partial);
                    if !unknown.iter().any(|u| u == word) {
                        unknown.push(word.to_owned());
                    }
                }
            }
        }
        out.push_str(rest);
        Converted { text: out, unknown }
    }

    /// 1 つの英数字のかたまりを読む。読めない部分が残ったら `Err`（途中まで読んだもの）。
    fn read_word(&self, word: &str) -> Result<String, String> {
        if let Some(reading) = self.words.get(&word.to_ascii_lowercase()) {
            return Ok(reading.clone());
        }
        let mut out = String::new();
        let mut ok = true;
        let parts = parts(word);
        for (i, part) in parts.iter().enumerate() {
            match part {
                Part::Alpha(alpha) => match self.read_alpha(alpha) {
                    Some(reading) => out.push_str(&reading),
                    None => {
                        out.push_str(alpha);
                        ok = false;
                    }
                },
                Part::Digits(digits) => out.push_str(digits),
                // 単語どうしをつなぐハイフン・下線は読まない（real-time → リアルタイム）
                Part::Symbol('-' | '_') => {}
                Part::Symbol('.') => {
                    let between_words = matches!(parts.get(i + 1), Some(Part::Alpha(_)))
                        && matches!(i.checked_sub(1).map(|j| &parts[j]), Some(Part::Alpha(_)));
                    out.push_str(if between_words { "ドット" } else { "." });
                }
                Part::Symbol('+') => out.push_str("プラス"),
                Part::Symbol('#') => out.push_str("シャープ"),
                Part::Symbol(c) => out.push(*c),
            }
        }
        if ok { Ok(out) } else { Err(out) }
    }

    fn read_alpha(&self, alpha: &str) -> Option<String> {
        if let Some(reading) = self.words.get(&alpha.to_ascii_lowercase()) {
            return Some(reading.clone());
        }
        // PowerShell → Power + Shell
        let camel = camel_parts(alpha);
        if camel.len() > 1 {
            let readings: Option<Vec<String>> = camel.iter().map(|p| self.read_alpha(p)).collect();
            if let Some(readings) = readings {
                return Some(readings.concat());
            }
        }
        if alpha.len() <= ACRONYM_MAX && alpha.chars().all(|c| c.is_ascii_uppercase()) {
            return Some(
                alpha
                    .bytes()
                    .map(|b| LETTERS[(b - b'A') as usize])
                    .collect(),
            );
        }
        None
    }

    /// 複数語の項目を、語の切れ目に合うところだけ置き換える。
    fn replace_phrases(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for (phrase, reading) in &self.phrases {
            let mut out = String::with_capacity(text.len());
            let lower = text.to_ascii_lowercase(); // ASCII だけ変えるのでバイト位置は同じ
            let mut last = 0;
            let mut from = 0;
            while let Some(i) = lower[from..].find(phrase.as_str()).map(|i| from + i) {
                let end = i + phrase.len();
                let boundary_before = !text[..i].ends_with(|c: char| c.is_ascii_alphanumeric());
                let boundary_after = !text[end..].starts_with(|c: char| c.is_ascii_alphanumeric());
                if boundary_before && boundary_after {
                    out.push_str(&text[last..i]);
                    out.push_str(reading);
                    last = end;
                }
                from = end;
            }
            out.push_str(&text[last..]);
            text = out;
        }
        text
    }
}

/// 英数字のかたまりの長さ。途中の `. - _ ' + #` は、後ろに英数字が続くとき（`+ #` は末尾でも）含める。
fn word_len(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let next_alnum = bytes.get(i + 1).is_some_and(u8::is_ascii_alphanumeric);
        let take = b.is_ascii_alphanumeric()
            || (matches!(b, b'.' | b'-' | b'_' | b'\'') && next_alnum)
            || matches!(b, b'+' | b'#');
        if !take {
            break;
        }
        i += 1;
    }
    i
}

#[derive(Debug)]
enum Part<'a> {
    Alpha(&'a str),
    Digits(&'a str),
    Symbol(char),
}

fn parts(word: &str) -> Vec<Part<'_>> {
    let mut out = Vec::new();
    let mut rest = word;
    while let Some(c) = rest.chars().next() {
        let len = if c.is_ascii_alphabetic() {
            rest.find(|c: char| !c.is_ascii_alphabetic() && c != '\'')
                .unwrap_or(rest.len())
        } else if c.is_ascii_digit() {
            rest.find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(rest.len())
        } else {
            c.len_utf8()
        };
        let (part, tail) = rest.split_at(len);
        out.push(if c.is_ascii_alphabetic() {
            Part::Alpha(part)
        } else if c.is_ascii_digit() {
            Part::Digits(part)
        } else {
            Part::Symbol(c)
        });
        rest = tail;
    }
    out
}

/// 大文字の切れ目で分ける。`HTTPServer` → HTTP + Server、`GitHub` → Git + Hub。
fn camel_parts(alpha: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = alpha.char_indices().collect();
    let mut cuts = vec![0];
    for w in 1..chars.len() {
        let (i, c) = chars[w];
        let prev = chars[w - 1].1;
        let next = chars.get(w + 1).map(|&(_, c)| c);
        let lower_to_upper = prev.is_ascii_lowercase() && c.is_ascii_uppercase();
        let acronym_end = prev.is_ascii_uppercase()
            && c.is_ascii_uppercase()
            && next.is_some_and(|n| n.is_ascii_lowercase());
        if lower_to_upper || acronym_end {
            cuts.push(i);
        }
    }
    cuts.push(alpha.len());
    cuts.windows(2).map(|w| &alpha[w[0]..w[1]]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 配布している例の辞書（`readings.example.toml`）を使う。
    fn example() -> Readings {
        Readings::from_toml(include_str!("../../../readings.example.toml")).unwrap()
    }

    fn read(text: &str) -> String {
        example().apply(text).text
    }

    #[test]
    fn dictionary_words() {
        assert_eq!(read("GitHubのPowerShellで"), "ギットハブのパワーシェルで");
        assert_eq!(read("Node.js と Rust"), "ノードジェイエス と ラスト");
    }

    #[test]
    fn camel_case_and_acronyms() {
        assert_eq!(read("HTTPServer"), "エイチティーティーピーサーバー");
        assert_eq!(read("GPUとVRAM"), "ジーピーユーとブイアールエーエム");
    }

    #[test]
    fn symbols_and_digits() {
        assert_eq!(read("real-time"), "リアルタイム");
        assert_eq!(read("C++とC#"), "シープラスプラスとシーシャープ");
        assert_eq!(read("UTF-8"), "ユーティーエフ8");
        assert_eq!(read("mp3を3.5秒"), "エムピースリーを3.5秒");
    }

    #[test]
    fn phrases_respect_word_boundaries() {
        assert_eq!(read("Pull Requestを出す"), "プルリクエストを出す");
        assert_eq!(read("Claude Codeで"), "クロードコードで");
    }

    #[test]
    fn unknown_words_are_reported() {
        let converted = example().apply("Zebra と zebra と GitHub");
        assert_eq!(converted.text, "Zebra と zebra と ギットハブ");
        assert_eq!(converted.unknown, ["Zebra", "zebra"]);
    }

    #[test]
    fn dictionary_is_only_what_is_given() {
        let readings =
            Readings::from_toml("github = \"ギットハブさん\"\n\"zebra crossing\" = \"横断歩道\"")
                .unwrap();
        assert_eq!(readings.apply("GitHub").text, "ギットハブさん");
        assert_eq!(readings.apply("zebra crossing").text, "横断歩道");
        // 辞書に書いていない語は英語のまま
        assert_eq!(readings.apply("PowerShell").unknown, ["PowerShell"]);
    }

    #[test]
    fn camel_split() {
        assert_eq!(camel_parts("PowerShell"), ["Power", "Shell"]);
        assert_eq!(camel_parts("HTTPServer"), ["HTTP", "Server"]);
        assert_eq!(camel_parts("iPhone"), ["i", "Phone"]);
    }
}
