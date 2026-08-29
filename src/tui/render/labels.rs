//! Gist, row, and time labels.
//!
//! The marks below are the whole vocabulary the row labels may draw from — see
//! [`docs/design.md`](../../../docs/design.md). Every one is single-width on purpose: a
//! double-width glyph misaligns the columns beside it and forces width special-cases into
//! `text_fit`. A meaning that would need an emoji gets a short word instead (`3 files`),
//! never a picture.

use super::*;

/// You starred this gist.
pub(crate) const MARK_STARRED: char = '★';
/// Stargazer count that follows.
pub(crate) const MARK_STARGAZERS: char = '☆';
/// This gist is a fork, or the fork count that follows.
pub(crate) const MARK_FORK: char = '⑂';
/// This local file and gist file are a pinned pair.
pub(crate) const MARK_PINNED: char = '↔';

/// `N files` / `1 file` — a count the user reads, in place of a file glyph.
pub(crate) fn file_count_label(count: usize) -> String {
    if count == 1 {
        "1 file".to_string()
    } else {
        format!("{count} files")
    }
}

/// `N comments` / `1 comment`, for the rows and info lines that surface a non-zero count.
pub(crate) fn comment_count_label(count: u32) -> String {
    if count == 1 {
        "1 comment".to_string()
    } else {
        format!("{count} comments")
    }
}

/// A count suffix for a list title: `(N)` normally, or `(shown/total)` when a filter has
/// narrowed the list (`shown < total`). Extends the existing `Files (N)` / `Comments (N)`
/// convention to the other panes consistently.
pub(crate) fn count_label(shown: usize, total: usize) -> String {
    if shown < total {
        format!("({shown}/{total})")
    } else {
        format!("({total})")
    }
}

pub(crate) fn gist_badge_prefix(starred: bool, forked: bool) -> String {
    let mut prefix = String::new();
    if starred {
        prefix.push(MARK_STARRED);
        prefix.push(' ');
    }
    if forked {
        prefix.push(MARK_FORK);
        prefix.push(' ');
    }
    prefix
}

/// Fixed-width badge column (★ starred, ⑂ forked, or blank) for the Gist manager row, so the
/// segment that follows — the owner prefix, then the description — starts at the same column
/// whether or not a row carries a badge (issue #347). Distinct from [`gist_badge_prefix`],
/// which the List screen's Gist pane uses and which stays variable-width there.
pub(crate) fn gist_manager_badge_prefix(starred: bool, forked: bool) -> String {
    let star = if starred { MARK_STARRED } else { ' ' };
    let fork = if forked { MARK_FORK } else { ' ' };
    format!("{star}{fork} ")
}

/// Width of the abbreviated-id column used by [`gist_group_row_label`] and the Pins row —
/// long enough to disambiguate at a glance, fixed so a legacy (shorter) gist id still lines up
/// with the columns that follow it (issue #347).
pub(crate) const SHORT_ID_WIDTH: usize = 7;

/// A fixed-width, left-aligned abbreviation of a gist id — the id stays reachable inline
/// without dominating the row the way the full 32-character id did.
pub(crate) fn short_gist_id(id: &str) -> String {
    let short: String = id.chars().take(SHORT_ID_WIDTH).collect();
    format!("{short:<SHORT_ID_WIDTH$}")
}

pub(crate) fn gist_owner_prefix(group: &GistGroup, current_user: Option<&str>) -> String {
    if group.owner_login.is_empty() {
        return String::new();
    }
    if current_user == Some(group.owner_login.as_str()) {
        return String::new();
    }
    format!("@{}  ", group.owner_login)
}

pub(crate) fn gist_group_row_label(
    g: &GistGroup,
    now: u64,
    sort: GistGroupSort,
    counts: (u32, u32, u32),
    starred: bool,
    current_user: Option<&str>,
) -> String {
    let (comments, stars, forks) = counts;
    let desc = if g.description.trim().is_empty() {
        "(no description)".to_string()
    } else {
        g.description.clone()
    };
    // Visibility is dropped from the row — it's surfaced by the `v` filter, the title's
    // `type:` label, and the detail view. Age is the last column, so its position carries
    // the meaning and it needs no label; it tracks the active sort key (created vs updated)
    // so the column the rows are ordered by is the one shown, as a single largest unit.
    let timestamp = match sort {
        GistGroupSort::Updated => &g.updated_at,
        GistGroupSort::Created => &g.created_at,
    };
    let age = crate::domain::parse_rfc3339_to_unix(timestamp)
        .map(|t| crate::domain::humanize_age(now as i64 - t as i64))
        .unwrap_or_else(|| "?".into());
    // Only surface metadata when it is non-zero, so the common quiet rows stay quiet.
    let stars_seg = if stars > 0 {
        format!("  {MARK_STARGAZERS} {stars}")
    } else {
        String::new()
    };
    let forks_seg = if forks > 0 {
        format!("  {MARK_FORK} {forks}")
    } else {
        String::new()
    };
    let comments_seg = if comments > 0 {
        format!("  {}", comment_count_label(comments))
    } else {
        String::new()
    };
    format!(
        "{}{}{}  #{}  {}{}{}{}  {}",
        gist_manager_badge_prefix(starred, g.fork_of_id.is_some()),
        gist_owner_prefix(g, current_user),
        desc,
        short_gist_id(&g.id),
        file_count_label(g.file_count),
        stars_seg,
        forks_seg,
        comments_seg,
        age
    )
}

pub(crate) fn gist_info_counts_seg(comments: u32, stars: u32, forks: u32) -> String {
    let mut parts = Vec::new();
    if stars > 0 {
        parts.push(format!("{MARK_STARGAZERS} {stars}"));
    }
    if forks > 0 {
        parts.push(format!("{MARK_FORK} {forks}"));
    }
    if comments > 0 {
        parts.push(comment_count_label(comments));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{} · ", parts.join(" · "))
    }
}

/// One-line info summary for the detail header.
pub(crate) fn gist_info_line(
    group: &GistGroup,
    now: u64,
    current_user: Option<&str>,
    starred: bool,
    counts: (u32, u32, u32),
) -> String {
    let (comments, stars, forks) = counts;
    let star_seg = if starred {
        format!("{MARK_STARRED} starred · ")
    } else {
        String::new()
    };
    let vis = if group.public { "public" } else { "secret" };
    let owner_seg = gist_owner_prefix(group, current_user);
    let counts_seg = gist_info_counts_seg(comments, stars, forks);
    let created = crate::domain::parse_rfc3339_to_unix(&group.created_at)
        .map(|t| crate::domain::humanize_age(now as i64 - t as i64))
        .unwrap_or_else(|| "?".into());
    let updated = crate::domain::parse_rfc3339_to_unix(&group.updated_at)
        .map(|t| crate::domain::humanize_age(now as i64 - t as i64))
        .unwrap_or_else(|| "?".into());
    // The file count lives in the "Files (N)" section header below, so it's omitted here.
    // The detail view has room, so show the full gist id (not a truncated prefix).
    let fork_seg = group
        .fork_of_id
        .as_deref()
        .map(|id| format!("fork of {id} · "))
        .unwrap_or_default();
    format!(
        "{star_seg}{owner_seg}{vis} · {counts_seg}created {created} · updated {updated} · {fork_seg}{}",
        group.id
    )
}

/// Current Unix time in seconds (saturating to 0 before the epoch); used for relative-age labels.
pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Info + file-list block for a gist (reused as the compaction-confirm background).
/// First visible file index so `cursor` stays within a `visible_rows`-high window over
/// `count` files. Returns 0 when everything fits or `visible_rows == 0`.
pub(crate) fn file_list_scroll(cursor: usize, visible_rows: usize, count: usize) -> usize {
    if visible_rows == 0 || count <= visible_rows || cursor < visible_rows {
        return 0;
    }
    (cursor + 1).saturating_sub(visible_rows)
}

/// Build the numbered file rows for the gist's file list (detail Files tab and the
/// compaction-confirm background). The first nine files are numbered to match the 1–9 preview
/// keys; the rest are bulleted. With `highlight_cursor`, the `cursor` row is reverse-styled.
/// Windows to `visible_rows` rows starting at `offset`.
pub(crate) fn file_rows(
    files: &[String],
    cursor: usize,
    offset: usize,
    visible_rows: usize,
    highlight_cursor: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    for (i, f) in files
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible_rows.max(1))
    {
        let marker = if i < 9 {
            format!("{}.", i + 1)
        } else {
            "·".to_string()
        };
        if highlight_cursor && i == cursor {
            rows.push(Line::from(Span::styled(
                format!("▸ {marker} {f}"),
                Style::default()
                    .fg(theme.fg_on_accent)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )));
        } else {
            rows.push(Line::from(format!("  {marker} {f}")));
        }
    }
    rows
}

/// Compose the full list-row string (including pin mark) that paint and hscroll max must share.
pub(crate) fn marked_row_text(base: String, mark: crate::ranking::MatchMark) -> String {
    match mark {
        crate::ranking::MatchMark::Pinned => format!("{MARK_PINNED} {base}"),
        crate::ranking::MatchMark::ExactFilename | crate::ranking::MatchMark::None => base,
    }
}

/// Gist file-list row **without** the live star mark (fork badge still applied). Used as the
/// shared base for paint/hscroll; star is layered in [`gist_row_display`] so both stay aligned.
pub(crate) fn gist_row_label(g: &RankedGistFile, view: GistView) -> String {
    let base = match view {
        GistView::Description => {
            if g.file.description.trim().is_empty() {
                g.file.filename.clone()
            } else {
                format!("{} — {}", g.file.filename, g.file.description)
            }
        }
        GistView::Id => format!("{} / {}", g.file.gist_id, g.file.filename),
    };
    format!("{}{}", gist_badge_prefix(false, g.file.is_fork()), base)
}

/// Civil date (year, month, day) from a day count since the Unix epoch — Howard Hinnant's
/// algorithm. UTC, leap-second agnostic (fine for display).
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub(crate) fn format_unix_utc(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86400));
    let rem = secs.rem_euclid(86400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60
    )
}

pub(crate) fn file_mtime_label(path: &std::path::Path) -> String {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| format_unix_utc(d.as_secs() as i64))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Normalises the gist API's RFC3339 `updated_at` (e.g. `2026-06-08T11:06:18Z`) to
/// `2026-06-08 11:06 UTC` for display alongside the local file's mtime.
pub(crate) fn gist_time_label(updated_at: &str) -> String {
    if updated_at.is_empty() {
        "unknown".to_string()
    } else if updated_at.len() >= 16 {
        format!("{} UTC", updated_at[..16].replace('T', " "))
    } else {
        updated_at.to_string()
    }
}

// ---------------------------------------------------------------------------
// Pinned-sync helpers (Task 9 + Task 10)
// ---------------------------------------------------------------------------

pub(crate) fn diff_labels(
    local_path: Option<&std::path::Path>,
    gist: &GistFile,
) -> (String, String) {
    let local_name = local_path
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("(none)");
    let local_time = local_path
        .map(file_mtime_label)
        .unwrap_or_else(|| "—".to_string());
    let local_label = format!("local: {local_name} ({local_time})");
    let gist_label = format!(
        "gist {} / {} ({})",
        gist.gist_id,
        gist.filename,
        gist_time_label(&gist.updated_at)
    );
    (local_label, gist_label)
}

/// Orientation for the `Enter` diff preview, driven by the focused pane: focusing the gist
/// pane frames it as a *download* (old = local, new = gist), focusing the local pane frames
/// it as an *upload* (old = gist, new = local). The dedicated `d`/`u` actions keep their own
/// fixed orientation; this only affects the read-only preview.
pub(crate) fn preview_diff_text(
    upload_orientation: bool,
    local_label: &str,
    local_content: &str,
    gist_label: &str,
    remote: &str,
    ignore_trailing_newline: bool,
) -> String {
    if upload_orientation {
        crate::diff::unified_diff(
            gist_label,
            remote,
            local_label,
            local_content,
            ignore_trailing_newline,
        )
    } else {
        crate::diff::unified_diff(
            local_label,
            local_content,
            gist_label,
            remote,
            ignore_trailing_newline,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_list_scroll_keeps_cursor_visible() {
        // count <= visible: no scroll.
        assert_eq!(file_list_scroll(0, 5, 3), 0);
        assert_eq!(file_list_scroll(2, 5, 3), 0);
        // cursor within the first window: no scroll.
        assert_eq!(file_list_scroll(2, 5, 20), 0);
        assert_eq!(file_list_scroll(4, 5, 20), 0);
        // cursor past the window: scroll so cursor is the last visible row.
        assert_eq!(file_list_scroll(5, 5, 20), 1);
        assert_eq!(file_list_scroll(19, 5, 20), 15);
        // visible_rows == 0: never panic, offset 0.
        assert_eq!(file_list_scroll(19, 0, 20), 0);
    }

    #[test]
    fn count_label_plain_unless_filtered() {
        assert_eq!(count_label(12, 12), "(12)");
        assert_eq!(count_label(0, 0), "(0)");
        // Filtered: fewer shown than total.
        assert_eq!(count_label(3, 12), "(3/12)");
    }

    #[test]
    fn gist_row_label_switches_with_view() {
        let g = RankedGistFile {
            file: GistFile {
                description: "My Ghostty config".into(),
                public: true,
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("abc", "config")
            },
            mark: crate::ranking::MatchMark::None,
        };
        assert_eq!(
            gist_row_label(&g, GistView::Description),
            "config — My Ghostty config"
        );
        assert_eq!(gist_row_label(&g, GistView::Id), "abc / config");
    }

    #[test]
    fn preview_diff_text_flips_with_focus() {
        // Download orientation (gist pane focused): old = local, new = gist.
        let dl = preview_diff_text(false, "local: a", "old\n", "gist b", "new\n", false);
        assert!(dl.starts_with("--- local: a\n+++ gist b\n"));

        // Upload orientation (local pane focused): old = gist, new = local.
        let ul = preview_diff_text(true, "local: a", "old\n", "gist b", "new\n", false);
        assert!(ul.starts_with("--- gist b\n+++ local: a\n"));
    }

    #[test]
    fn format_unix_utc_known_instants() {
        assert_eq!(format_unix_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_unix_utc(1_780_656_360), "2026-06-05 10:46 UTC");
    }

    #[test]
    fn gist_time_label_normalises_rfc3339() {
        assert_eq!(
            gist_time_label("2026-06-08T11:06:18Z"),
            "2026-06-08 11:06 UTC"
        );
        assert_eq!(gist_time_label(""), "unknown");
        assert_eq!(gist_time_label("short"), "short");
    }

    #[test]
    fn marked_row_text_uses_match_mark_pin_prefix() {
        use crate::ranking::MatchMark;
        assert_eq!(marked_row_text("x".into(), MatchMark::Pinned), "↔ x");
        assert_eq!(marked_row_text("x".into(), MatchMark::ExactFilename), "x");
        assert_eq!(marked_row_text("x".into(), MatchMark::None), "x");
    }

    #[test]
    fn gist_row_label_falls_back_to_filename_when_description_empty() {
        let g = RankedGistFile {
            file: GistFile {
                description: "  ".into(),
                public: true,
                updated_at: "x".into(),
                created_at: "x".into(),
                ..GistFile::fixture("abc", "config")
            },
            mark: crate::ranking::MatchMark::None,
        };
        assert_eq!(gist_row_label(&g, GistView::Description), "config");
    }

    #[test]
    fn gist_group_row_age_tracks_active_sort() {
        let group = GistGroup {
            id: "g1".into(),
            description: "demo".into(),
            public: false,
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
            file_count: 2,
            owner_login: String::new(),
            fork_of_id: None,
        };
        let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
        // Sorting by updated shows the updated age (1 day ago); sorting by created shows the
        // created age (10 days ago → "1w"), so the age column matches the ordering key.
        let updated =
            gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 0), false, None);
        let created =
            gist_group_row_label(&group, now, GistGroupSort::Created, (0, 0, 0), false, None);
        assert!(updated.ends_with("  1d"), "{updated}");
        assert!(created.ends_with("  1w"), "{created}");
    }

    #[test]
    fn gist_group_row_shows_comment_count_only_when_present() {
        let group = GistGroup {
            id: "g1".into(),
            description: "demo".into(),
            public: false,
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
            file_count: 2,
            owner_login: String::new(),
            fork_of_id: None,
        };
        let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
        assert!(
            !gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 0), false, None)
                .contains("comment")
        );
        assert!(
            gist_group_row_label(&group, now, GistGroupSort::Updated, (3, 0, 0), false, None)
                .contains("3 comments")
        );
    }

    #[test]
    fn gist_group_row_shows_foreign_owner() {
        let group = GistGroup {
            id: "g1".into(),
            description: "demo".into(),
            public: true,
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
            file_count: 1,
            owner_login: "karpathy".into(),
            fork_of_id: None,
        };
        let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
        let foreign = gist_group_row_label(
            &group,
            now,
            GistGroupSort::Updated,
            (0, 0, 0),
            false,
            Some("me"),
        );
        assert!(foreign.contains("@karpathy"));
        let own = gist_group_row_label(
            &group,
            now,
            GistGroupSort::Updated,
            (0, 0, 0),
            false,
            Some("karpathy"),
        );
        assert!(!own.contains("@karpathy"));
    }

    #[test]
    fn gist_group_row_shows_fork_marker_only_when_present() {
        let group = GistGroup {
            id: "g1".into(),
            description: "demo".into(),
            public: false,
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
            file_count: 2,
            owner_login: String::new(),
            fork_of_id: None,
        };
        let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
        assert!(
            !gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 0), false, None)
                .contains('⑂')
        );
        assert!(
            gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 2), false, None)
                .contains("⑂ 2")
        );
    }

    #[test]
    fn gist_group_row_shows_star_marker_only_when_present() {
        let group = GistGroup {
            id: "g1".into(),
            description: "demo".into(),
            public: false,
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
            file_count: 2,
            owner_login: String::new(),
            fork_of_id: None,
        };
        let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
        assert!(
            !gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 0), false, None)
                .contains('☆')
        );
        assert!(
            gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 3, 0), false, None)
                .contains("☆ 3")
        );
    }

    #[test]
    fn gist_group_row_description_leads_and_id_is_abbreviated() {
        let group = GistGroup {
            id: "abcdef0123456789abcdef0123456789".into(),
            description: "My cool gist".into(),
            public: false,
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
            file_count: 2,
            owner_login: String::new(),
            fork_of_id: None,
        };
        let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
        let row = gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 0), false, None);
        assert!(
            row.trim_start().starts_with("My cool gist"),
            "description should lead the row, got {row}"
        );
        assert!(!row.contains(&group.id), "full id must not appear: {row}");
        assert!(
            row.contains(&format!("#{}", &group.id[..7])),
            "abbreviated id should still be reachable inline: {row}"
        );
    }

    #[test]
    fn gist_group_row_badge_column_is_fixed_width() {
        let group = GistGroup {
            id: "g1".into(),
            description: "demo".into(),
            public: false,
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
            file_count: 1,
            owner_login: String::new(),
            fork_of_id: None,
        };
        let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
        let unbadged =
            gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 0), false, None);
        let starred =
            gist_group_row_label(&group, now, GistGroupSort::Updated, (0, 0, 0), true, None);
        // Compare by char count, not byte offset — `★` is multi-byte, so a byte-offset comparison
        // would report misalignment even though the two rows line up on screen.
        let char_col = |s: &str| s.find("demo").map(|byte_idx| s[..byte_idx].chars().count());
        assert_eq!(
            char_col(&unbadged),
            char_col(&starred),
            "description column must align with and without a badge: {unbadged:?} vs {starred:?}"
        );
    }

    #[test]
    fn gist_group_row_legacy_short_id_still_aligns() {
        let short = GistGroup {
            id: "abc12".into(),
            description: "demo".into(),
            public: false,
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
            file_count: 1,
            owner_login: String::new(),
            fork_of_id: None,
        };
        let long = GistGroup {
            id: "abcdef0123456789".into(),
            ..short.clone()
        };
        let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
        let short_row =
            gist_group_row_label(&short, now, GistGroupSort::Updated, (0, 0, 0), false, None);
        let long_row =
            gist_group_row_label(&long, now, GistGroupSort::Updated, (0, 0, 0), false, None);
        assert_eq!(
            short_row.find("1 file"),
            long_row.find("1 file"),
            "the file count must land at the same column regardless of id length: \
         {short_row:?} vs {long_row:?}"
        );
    }

    #[test]
    fn gist_info_line_shows_counts_when_nonzero() {
        let group = GistGroup {
            id: "616796de59282c8bfdae3005511c588e".into(),
            description: "demo".into(),
            public: true,
            updated_at: "2026-06-10T00:00:00Z".into(),
            created_at: "2026-06-01T00:00:00Z".into(),
            file_count: 1,
            owner_login: String::new(),
            fork_of_id: None,
        };
        let now = crate::domain::parse_rfc3339_to_unix("2026-06-11T00:00:00Z").unwrap();
        let quiet = gist_info_line(&group, now, None, false, (0, 0, 0));
        assert!(!quiet.contains('☆'));
        assert!(!quiet.contains('⑂'));
        assert!(!quiet.contains("comment"));

        let rich = gist_info_line(&group, now, None, true, (2, 3, 1));
        assert!(rich.starts_with("★ starred · "));
        assert!(rich.contains("☆ 3 · ⑂ 1 · 2 comments"));
        assert!(rich.contains(&group.id));
    }
}
