/// Markdown の記号、URL、コードの塊を取り除き、読み上げる文だけにする。
///
/// - コードの塊（```）は丸ごと飛ばす。インラインコードは中身だけ残す
/// - 見出し・箇条書き・引用の記号、強調（`**` `__` `~~`）を消す
/// - `[文字](URL)` は文字だけ残し、むき出しの URL は消す
/// - 表の `|` は読点にする
pub fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_fence = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push('\n');
            continue;
        }
        if in_fence {
            continue;
        }
        if is_table_rule(trimmed) {
            continue;
        }
        let line = strip_line_prefix(trimmed);
        let line = strip_links(line);
        let line = strip_urls(&line);
        let line: String = line
            .replace("**", "")
            .replace("__", "")
            .replace("~~", "")
            .chars()
            .filter(|&c| c != '`')
            .collect();
        let line = table_cells(&line);
        out.push_str(line.trim());
        out.push('\n');
    }
    out
}

/// 見出し・箇条書き・番号付きリスト・引用の記号を外す。
fn strip_line_prefix(line: &str) -> &str {
    let mut line = line;
    loop {
        let before = line;
        line = line.trim_start_matches('>').trim_start();
        line = line.trim_start_matches('#').trim_start();
        for bullet in ["- ", "* ", "+ ", "・"] {
            if let Some(rest) = line.strip_prefix(bullet) {
                line = rest.trim_start();
            }
        }
        // 「1. 」「12) 」
        let digits = line.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 {
            let rest = &line[digits..];
            if let Some(rest) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
                line = rest.trim_start();
            }
        }
        if line == before {
            return line;
        }
    }
}

/// `[文字](URL)` を `文字` にする。画像 `![説明](URL)` は説明だけ残す。
fn strip_links(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](").map(|i| open + i) else {
            break;
        };
        let Some(end) = rest[close..].find(')').map(|i| close + i) else {
            break;
        };
        let before = rest[..open].strip_suffix('!').unwrap_or(&rest[..open]);
        out.push_str(before);
        out.push_str(&rest[open + 1..close]);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// `http://` `https://` で始まる語を消す。
fn strip_urls(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    loop {
        let found = ["https://", "http://"]
            .iter()
            .filter_map(|p| rest.find(p))
            .min();
        let Some(start) = found else { break };
        out.push_str(&rest[..start]);
        let end = rest[start..]
            .find(|c: char| c.is_whitespace() || "）)」』>\"'".contains(c) || !c.is_ascii())
            .map_or(rest.len(), |i| start + i);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// `|---|:--:|` のような表の区切り行。
fn is_table_rule(line: &str) -> bool {
    line.starts_with('|') && line.chars().all(|c| "|-: ".contains(c))
}

/// 表の行 `| a | b |` を `a、b` にする。
fn table_cells(line: &str) -> String {
    if !line.trim_start().starts_with('|') {
        return line.to_owned();
    }
    line.split('|')
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .collect::<Vec<_>>()
        .join("、")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_markdown_syntax() {
        let input = "# 見出し\n\n- **太字**の項目\n1. 番号付き\n> 引用です\n`code` を使う";
        let lines: Vec<_> = clean(input)
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect();
        assert_eq!(
            lines,
            [
                "見出し",
                "太字の項目",
                "番号付き",
                "引用です",
                "code を使う"
            ]
        );
    }

    #[test]
    fn skips_code_fences() {
        let input = "前\n```rust\nfn main() {}\n```\n後";
        let text: Vec<_> = clean(input).split_whitespace().map(str::to_owned).collect();
        assert_eq!(text, ["前", "後"]);
    }

    #[test]
    fn links_and_urls() {
        assert_eq!(
            clean("[公式サイト](https://example.com)を見る").trim(),
            "公式サイトを見る"
        );
        assert_eq!(
            clean("詳しくは https://example.com/a?b=c を参照").trim(),
            "詳しくは  を参照"
        );
        assert_eq!(
            clean("URL（https://example.com）です").trim(),
            "URL（）です"
        );
    }

    #[test]
    fn tables() {
        let input = "| 名前 | 内容 |\n|---|---|\n| mozu | 声 |";
        let lines: Vec<_> = clean(input).lines().map(str::to_owned).collect();
        assert_eq!(lines, ["名前、内容", "mozu、声"]);
    }
}
