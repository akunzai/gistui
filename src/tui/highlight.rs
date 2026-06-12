//! Render-layer syntax highlighting (issue #69).
//!
//! Pure mapping from file content to coloured ratatui spans via the lean `synoptic` crate. It
//! lives in the render layer to keep the domain/render split: nothing here mutates app state or
//! touches IO. Unknown extensions fall through to plain text, and the whole feature is gated by
//! `AppState::syntax_highlight` (off when `NO_COLOR` is set) at the call sites in `render`.

use ratatui::style::{Color, Style};
use ratatui::text::Span;
use synoptic::{from_extension, Highlighter, TokOpt};

/// Tab display width handed to synoptic; it expands `\t` to this many spaces in highlighted text.
const TAB_WIDTH: usize = 4;

/// Map a synoptic token kind to a terminal colour. Unknown kinds → `None`, rendered as plain
/// text. Greens/reds are deliberately limited so highlighted diff-context lines don't read as
/// added/removed lines. Synoptic's kinds across its bundled grammars are: comment, string,
/// keyword, digit, boolean, function, struct, namespace, attribute, header.
fn token_color(kind: &str) -> Option<Color> {
    Some(match kind {
        "comment" => Color::DarkGray,
        "string" => Color::Green,
        "keyword" => Color::Magenta,
        "digit" | "boolean" => Color::Cyan,
        "function" => Color::Blue,
        "struct" | "namespace" => Color::Yellow,
        "attribute" | "header" => Color::Cyan,
        _ => return None,
    })
}

/// Highlight a whole buffer, correct across multi-line strings/comments (synoptic tracks state
/// over the run). Returns one span vector per input line, 1:1 with `lines`. `ext` selects the
/// grammar; an unsupported extension yields all-plain spans so callers always get a usable
/// result of the same shape.
pub fn highlight_buffer(ext: &str, lines: &[String]) -> Vec<Vec<Span<'static>>> {
    let Some(mut highlighter): Option<Highlighter> = from_extension(ext, TAB_WIDTH) else {
        return lines.iter().map(|l| vec![Span::raw(l.clone())]).collect();
    };
    highlighter.run(lines);
    lines
        .iter()
        .enumerate()
        .map(|(y, line)| spans_from_tokens(&highlighter.line(y, line)))
        .collect()
}

/// Convert one line's synoptic tokens into styled spans, colouring recognised kinds and leaving
/// the rest plain. An empty token list (a blank line) yields a single empty span so the line
/// still occupies a row.
fn spans_from_tokens(tokens: &[TokOpt]) -> Vec<Span<'static>> {
    if tokens.is_empty() {
        return vec![Span::raw(String::new())];
    }
    tokens
        .iter()
        .map(|tok| match tok {
            TokOpt::Some(text, kind) => match token_color(kind) {
                Some(color) => Span::styled(text.clone(), Style::default().fg(color)),
                None => Span::raw(text.clone()),
            },
            TokOpt::None(text) => Span::raw(text.clone()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The concatenated span text must reproduce the source line exactly (no characters dropped
    /// or reordered) — highlighting only re-styles, never rewrites content.
    fn joined(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn known_language_colours_keywords() {
        let lines = vec!["fn main() {}".to_string()];
        let out = highlight_buffer("rs", &lines);
        assert_eq!(out.len(), 1);
        assert_eq!(joined(&out[0]), "fn main() {}");
        // `fn` is a Rust keyword → magenta; at least one span must be coloured.
        assert!(out[0]
            .iter()
            .any(|s| s.style.fg == Some(Color::Magenta) && s.content.contains("fn")));
    }

    #[test]
    fn unknown_extension_renders_plain() {
        let lines = vec!["fn main() {}".to_string()];
        let out = highlight_buffer("unknown_ext_zzz", &lines);
        assert_eq!(joined(&out[0]), "fn main() {}");
        // No span carries a foreground colour for an unsupported grammar.
        assert!(out[0].iter().all(|s| s.style.fg.is_none()));
    }

    #[test]
    fn blank_line_stays_a_row() {
        let out = highlight_buffer("rs", &["".to_string()]);
        assert_eq!(out.len(), 1);
        assert_eq!(joined(&out[0]), "");
    }

    #[test]
    fn token_color_maps_known_kinds_and_ignores_others() {
        assert_eq!(token_color("comment"), Some(Color::DarkGray));
        assert_eq!(token_color("string"), Some(Color::Green));
        assert_eq!(token_color("keyword"), Some(Color::Magenta));
        assert_eq!(token_color("operator"), None);
    }
}
