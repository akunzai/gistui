use crate::config::ThemeChoice;
use ratatui::style::{Color, Style};

/// Syntax-highlight token colours (issue #69), grouped so the whole set swaps with the theme.
/// The dark set are bright ANSI hues tuned for a terminal-native background; the light set are
/// fixed 256-colour dark shades that keep contrast on the light-grey canvas (bright ANSI hues
/// wash out there). Kinds that share a hue (digit/boolean, attribute/header) map to one field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxPalette {
    /// `comment` tokens.
    pub comment: Color,
    /// `string` tokens.
    pub string: Color,
    /// `keyword` tokens.
    pub keyword: Color,
    /// `digit` / `boolean` literals.
    pub literal: Color,
    /// `function` tokens.
    pub function: Color,
    /// `struct` / `namespace` type names.
    pub type_name: Color,
    /// `attribute` / `header` tokens.
    pub attribute: Color,
}

impl SyntaxPalette {
    pub const DARK: SyntaxPalette = SyntaxPalette {
        comment: Color::DarkGray,
        string: Color::Green,
        keyword: Color::Magenta,
        literal: Color::Cyan,
        function: Color::Blue,
        type_name: Color::Yellow,
        attribute: Color::Cyan,
    };

    pub const LIGHT: SyntaxPalette = SyntaxPalette {
        comment: Color::Indexed(240),  // #585858 dim grey
        string: Color::Indexed(28),    // #008700 dark green
        keyword: Color::Indexed(90),   // #870087 dark magenta
        literal: Color::Indexed(30),   // #008787 dark teal
        function: Color::Indexed(19),  // #0000af dark blue
        type_name: Color::Indexed(94), // #875f00 dark amber
        attribute: Color::Indexed(30), // #008787 dark teal
    };
}

/// Semantic colour palette for the TUI. All render code uses these fields rather than
/// hard-coded `Color::*` values so that swapping themes only requires changing `AppState::theme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Canvas background — `Color::Reset` for dark (terminal native), `Color::White` for light.
    pub bg: Color,
    /// Default foreground text on `bg` — `Color::Reset` for dark, `Color::Black` for light.
    pub fg: Color,
    /// Primary accent: focused-pane borders, selection highlight background, overlay borders.
    pub accent: Color,
    /// Foreground drawn on top of an `accent`-coloured background.
    pub fg_on_accent: Color,
    /// Subdued text: unfocused borders, empty-list placeholders, footer separator.
    pub dim: Color,
    /// Footer hint key colour for write/sync actions (download, upload, sync, …).
    pub write_color: Color,
    /// Diff `+` insertion line colour.
    pub ins_color: Color,
    /// Diff `-` deletion line colour.
    pub del_color: Color,
    /// Diff header keyword colour for the "gist" label (`--- gist …`).
    pub gist_label_color: Color,
    /// Amber/yellow accent: the diff `local` header label and non-destructive write
    /// confirmations (create / upload).
    pub notice_color: Color,
    /// Syntax-highlight token colours for the diff context and preview panes.
    pub syntax: SyntaxPalette,
}

impl Theme {
    pub const DARK: Theme = Theme {
        bg: Color::Reset,
        fg: Color::Reset,
        accent: Color::Cyan,
        fg_on_accent: Color::Black,
        dim: Color::DarkGray,
        write_color: Color::Green,
        ins_color: Color::Green,
        del_color: Color::Red,
        gist_label_color: Color::Blue,
        notice_color: Color::Yellow,
        syntax: SyntaxPalette::DARK,
    };

    pub const LIGHT: Theme = Theme {
        bg: Color::Gray, // ANSI 7 — light-grey canvas
        fg: Color::Black,
        accent: Color::DarkGray, // ANSI 8 — selection bar, focused borders
        fg_on_accent: Color::White,
        dim: Color::DarkGray,
        // Bright ANSI Green / Blue wash out on the grey canvas; pin a fixed dark navy
        // (256-colour index 20, #0000d7) that keeps strong contrast regardless of the
        // terminal's ANSI palette.
        write_color: Color::Indexed(20),
        ins_color: Color::Indexed(20),
        del_color: Color::Indexed(124), // #af0000 dark red — bright ANSI red is too light here
        gist_label_color: Color::DarkGray,
        notice_color: Color::Indexed(94), // #875f00 dark amber
        syntax: SyntaxPalette::LIGHT,
    };

    pub fn for_choice(choice: ThemeChoice) -> Theme {
        match choice {
            ThemeChoice::Dark => Theme::DARK,
            ThemeChoice::Light => Theme::LIGHT,
        }
    }

    /// Base style for Block / List / Paragraph widgets: sets canvas `bg` and default `fg`
    /// so every cell drawn by the widget shows the theme background instead of the terminal's
    /// native colour. No-op for dark theme (`Color::Reset` = terminal default).
    pub fn base_style(self) -> Style {
        Style::default().bg(self.bg).fg(self.fg)
    }
}
