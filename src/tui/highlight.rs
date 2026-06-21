//! Render-layer syntax highlighting (issue #69).
//!
//! Pure mapping from file content to coloured ratatui spans via the lean `synoptic` crate. It
//! lives in the render layer to keep the domain/render split: nothing here mutates app state or
//! touches IO. Unknown extensions fall through to plain text, and the whole feature is gated by
//! `AppState::syntax_highlight` (off when `NO_COLOR` is set) at the call sites in `render`.

use super::theme::Theme;
use crate::lru::LruCache;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use synoptic::{from_extension, Highlighter, TokOpt};

/// Tab display width handed to synoptic; it expands `\t` to this many spaces in highlighted text.
const TAB_WIDTH: usize = 4;

/// Map a synoptic token kind to the active theme's colour. Unknown kinds → `None`, rendered as
/// plain text. The colours come from `theme.syntax` so the light theme can swap the bright ANSI
/// hues (which wash out on its grey canvas) for fixed dark shades. Synoptic's kinds across its
/// bundled grammars are: comment, string, keyword, digit, boolean, function, struct, namespace,
/// attribute, header.
fn token_color(kind: &str, theme: &Theme) -> Option<Color> {
    let s = &theme.syntax;
    Some(match kind {
        "comment" => s.comment,
        "string" => s.string,
        "keyword" => s.keyword,
        "digit" | "boolean" => s.literal,
        "function" => s.function,
        "struct" | "namespace" => s.type_name,
        "attribute" | "header" => s.attribute,
        _ => return None,
    })
}

thread_local! {
    /// Per-thread memo of highlighted buffers. The render loop re-highlights the same
    /// `diff_text`/preview content on every ~150ms frame even when only the scroll offset
    /// changed; synoptic tokenisation is the costly part, so caching it (keyed on the exact
    /// inputs that determine the output) turns a steady-state frame into a cheap clone.
    /// Capacity 16 comfortably holds the few buffers in play (current diff/preview + recents)
    /// and bounds memory. Render runs on the main thread, so a thread-local is sufficient.
    static HIGHLIGHT_CACHE: RefCell<LruCache<u64, Vec<Vec<Span<'static>>>>> =
        RefCell::new(LruCache::new(16));
}

/// Hash the full set of inputs `highlight_buffer` is a pure function of: the grammar (`ext`),
/// the content (`lines`), and the token palette (`theme.syntax`, the only theme data
/// `token_color` reads). Any change to these yields a different key, so a stale palette or
/// edited content can never serve a cached result.
fn highlight_key(ext: &str, lines: &[String], theme: &Theme) -> u64 {
    let mut hasher = DefaultHasher::new();
    ext.hash(&mut hasher);
    lines.hash(&mut hasher);
    theme.syntax.hash(&mut hasher);
    hasher.finish()
}

/// Highlight a whole buffer, correct across multi-line strings/comments (synoptic tracks state
/// over the run). Returns one span vector per input line, 1:1 with `lines`. `ext` selects the
/// grammar; an unsupported extension yields all-plain spans so callers always get a usable
/// result of the same shape. `theme` selects the token palette.
///
/// Transparently memoised (see `HIGHLIGHT_CACHE`): identical `(ext, lines, theme.syntax)`
/// returns the same spans without re-tokenising. The cache is an implementation detail — the
/// result is byte-for-byte what a direct computation would produce.
pub fn highlight_buffer(ext: &str, lines: &[String], theme: &Theme) -> Vec<Vec<Span<'static>>> {
    let key = highlight_key(ext, lines, theme);
    if let Some(hit) = HIGHLIGHT_CACHE.with(|c| c.borrow_mut().get(&key).cloned()) {
        return hit;
    }
    let computed = highlight_buffer_uncached(ext, lines, theme);
    HIGHLIGHT_CACHE.with(|c| c.borrow_mut().insert(key, computed.clone()));
    computed
}

/// The actual synoptic tokenisation, without the memo. Split out so [`highlight_buffer`] is a
/// thin cache front and tests can exercise the computation directly.
fn highlight_buffer_uncached(
    ext: &str,
    lines: &[String],
    theme: &Theme,
) -> Vec<Vec<Span<'static>>> {
    let Some(mut highlighter): Option<Highlighter> = from_extension(ext, TAB_WIDTH) else {
        return lines.iter().map(|l| vec![Span::raw(l.clone())]).collect();
    };
    highlighter.run(lines);
    lines
        .iter()
        .enumerate()
        .map(|(y, line)| spans_from_tokens(&highlighter.line(y, line), theme))
        .collect()
}

/// Convert one line's synoptic tokens into styled spans, colouring recognised kinds and leaving
/// the rest plain. An empty token list (a blank line) yields a single empty span so the line
/// still occupies a row.
fn spans_from_tokens(tokens: &[TokOpt], theme: &Theme) -> Vec<Span<'static>> {
    if tokens.is_empty() {
        return vec![Span::raw(String::new())];
    }
    tokens
        .iter()
        .map(|tok| match tok {
            TokOpt::Some(text, kind) => match token_color(kind, theme) {
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
        let out = highlight_buffer("rs", &lines, &Theme::DARK);
        assert_eq!(out.len(), 1);
        assert_eq!(joined(&out[0]), "fn main() {}");
        // `fn` is a Rust keyword → magenta; at least one span must be coloured.
        assert!(out[0]
            .iter()
            .any(|s| s.style.fg == Some(Color::Magenta) && s.content.contains("fn")));
    }

    #[test]
    fn memo_matches_uncached_and_does_not_corrupt_across_keys() {
        // The memo must be transparent: a cached result equals a direct computation, and
        // interleaving a different buffer must not contaminate an earlier key's entry.
        let a = vec!["fn a() {}".to_string()];
        let b = vec!["let b = 2; // note".to_string()];
        let direct_a = highlight_buffer_uncached("rs", &a, &Theme::DARK);

        let a1 = highlight_buffer("rs", &a, &Theme::DARK); // miss → compute + store
        let _b = highlight_buffer("rs", &b, &Theme::DARK); // different key
        let a2 = highlight_buffer("rs", &a, &Theme::DARK); // hit → must still be a's result
        assert_eq!(a1, direct_a);
        assert_eq!(a2, direct_a);
    }

    #[test]
    fn memo_keys_on_theme_palette() {
        // Same content + grammar but a different palette must not collide in the cache:
        // the light theme recolours tokens, so its spans differ from the dark theme's.
        let lines = vec!["let x = 1; // c".to_string()];
        let dark = highlight_buffer("rs", &lines, &Theme::DARK);
        let light = highlight_buffer("rs", &lines, &Theme::LIGHT);
        assert_ne!(dark, light);
    }

    #[test]
    fn unknown_extension_renders_plain() {
        let lines = vec!["fn main() {}".to_string()];
        let out = highlight_buffer("unknown_ext_zzz", &lines, &Theme::DARK);
        assert_eq!(joined(&out[0]), "fn main() {}");
        // No span carries a foreground colour for an unsupported grammar.
        assert!(out[0].iter().all(|s| s.style.fg.is_none()));
    }

    #[test]
    fn blank_line_stays_a_row() {
        let out = highlight_buffer("rs", &["".to_string()], &Theme::DARK);
        assert_eq!(out.len(), 1);
        assert_eq!(joined(&out[0]), "");
    }

    #[test]
    fn token_color_maps_known_kinds_and_ignores_others() {
        assert_eq!(token_color("comment", &Theme::DARK), Some(Color::DarkGray));
        assert_eq!(token_color("string", &Theme::DARK), Some(Color::Green));
        assert_eq!(token_color("keyword", &Theme::DARK), Some(Color::Magenta));
        assert_eq!(token_color("operator", &Theme::DARK), None);
    }

    #[test]
    fn light_theme_swaps_token_palette() {
        // Light theme must not reuse the bright ANSI hues that wash out on its grey canvas.
        assert_eq!(
            token_color("string", &Theme::LIGHT),
            Some(Color::Indexed(28))
        );
        assert_eq!(
            token_color("keyword", &Theme::LIGHT),
            Some(Color::Indexed(90))
        );
        assert_eq!(token_color("operator", &Theme::LIGHT), None);
    }
}
