/// 文のあとに入れる間。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pause {
    /// 読点などで長い文を切ったところ。ほぼ間を空けない。
    Comma,
    /// 文の終わり。
    Sentence,
    /// 段落（空行）の終わり。
    Paragraph,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sentence {
    pub text: String,
    pub pause: Pause,
}

const SENTENCE_ENDS: &[char] = &['。', '！', '？', '!', '?', '．'];
const COMMAS: &[char] = &['、', '，', ',', '；', ';', '：'];

/// 文の切れ目で区切る。1 つが `max_chars` を超えるときは読点で、それでも長ければ文字数で切る。
/// 短い文どうしは、合成の回数を減らすため `max_chars` までまとめる。
pub fn split(text: &str, max_chars: usize) -> Vec<Sentence> {
    let mut sentences = Vec::new();
    for paragraph in text.split("\n\n") {
        let mut pieces: Vec<Sentence> = Vec::new();
        for line in paragraph.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            for sentence in split_keep(line, SENTENCE_ENDS) {
                for piece in split_long(sentence, max_chars) {
                    pieces.push(piece);
                }
            }
            // 改行は文の切れ目として扱う。見出しや箇条書きのように句点のない行は、
            // 次の行と続けて読まれないよう「。」を補う
            if let Some(last) = pieces.last_mut() {
                last.pause = Pause::Sentence;
                let ends_sentence = last.text.ends_with(|c: char| {
                    SENTENCE_ENDS.contains(&c) || COMMAS.contains(&c) || "」』）)".contains(c)
                });
                if !ends_sentence {
                    last.text.push('。');
                }
            }
        }
        let mut merged = merge_short(pieces, max_chars);
        if let Some(last) = merged.last_mut() {
            last.pause = Pause::Paragraph;
        }
        sentences.extend(merged);
    }
    sentences
}

/// 区切り文字を前の側に残して分ける。
fn split_keep<'a>(text: &'a str, delimiters: &[char]) -> Vec<&'a str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if delimiters.contains(&c) {
            // 「？！」のように続く記号はまとめる
            let mut end = i + c.len_utf8();
            while let Some(&(j, next)) = chars.peek() {
                if delimiters.contains(&next) || "」』）)".contains(next) {
                    end = j + next.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            parts.push(&text[start..end]);
            start = end;
        }
    }
    if start < text.len() {
        parts.push(&text[start..]);
    }
    parts
        .into_iter()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect()
}

fn split_long(sentence: &str, max_chars: usize) -> Vec<Sentence> {
    if sentence.chars().count() <= max_chars {
        return vec![Sentence {
            text: sentence.to_owned(),
            pause: Pause::Sentence,
        }];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for part in split_keep(sentence, COMMAS) {
        if !current.is_empty() && current.chars().count() + part.chars().count() > max_chars {
            out.push(Sentence {
                text: std::mem::take(&mut current),
                pause: Pause::Comma,
            });
        }
        current.push_str(part);
        // 読点がなく、それでも長いときは文字数で切る
        while current.chars().count() > max_chars {
            let cut: String = current.chars().take(max_chars).collect();
            current = current.chars().skip(max_chars).collect();
            out.push(Sentence {
                text: cut,
                pause: Pause::Comma,
            });
        }
    }
    if !current.is_empty() {
        out.push(Sentence {
            text: current,
            pause: Pause::Sentence,
        });
    }
    out
}

fn merge_short(pieces: Vec<Sentence>, max_chars: usize) -> Vec<Sentence> {
    let mut out: Vec<Sentence> = Vec::new();
    for piece in pieces {
        if let Some(last) = out.last_mut()
            && last.pause == Pause::Sentence
            && last.text.chars().count() + piece.text.chars().count() <= max_chars
        {
            last.text.push_str(&piece.text);
            last.pause = piece.pause;
            continue;
        }
        out.push(piece);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(sentences: &[Sentence]) -> Vec<&str> {
        sentences.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn merges_short_sentences() {
        let s = split("こんにちは。元気？今日は晴れ！", 80);
        assert_eq!(texts(&s), ["こんにちは。元気？今日は晴れ！"]);
        assert_eq!(s[0].pause, Pause::Paragraph);
    }

    #[test]
    fn splits_at_sentence_end_when_too_long() {
        let s = split("あいうえお。かきくけこ。", 8);
        assert_eq!(texts(&s), ["あいうえお。", "かきくけこ。"]);
        assert_eq!(s[0].pause, Pause::Sentence);
    }

    #[test]
    fn long_sentence_splits_at_commas_then_chars() {
        let s = split("あいうえお、かきくけこ、さしすせそたちつてと", 6);
        assert_eq!(
            texts(&s),
            ["あいうえお、", "かきくけこ、", "さしすせそた", "ちつてと。"]
        );
        assert_eq!(s[0].pause, Pause::Comma);
    }

    #[test]
    fn paragraphs() {
        let s = split("一つ目。\n\n二つ目。", 80);
        assert_eq!(texts(&s), ["一つ目。", "二つ目。"]);
        assert_eq!(s[0].pause, Pause::Paragraph);
    }

    #[test]
    fn lines_without_punctuation_end_a_sentence() {
        let s = split("使い方\n説明です。\n項目", 80);
        assert_eq!(texts(&s), ["使い方。説明です。項目。"]);
    }

    #[test]
    fn keeps_closing_brackets_with_sentence() {
        let s = split("「そうなの？」と聞いた。", 7);
        assert_eq!(texts(&s), ["「そうなの？」", "と聞いた。"]);
    }
}
