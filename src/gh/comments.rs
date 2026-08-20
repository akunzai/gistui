//! Gist comment fetch/parse/pagination (issue #301).

use super::{parse_gh_gists, GhCommentUser};
use crate::actions::{run_command, CommandPlan, CommandRunner};
use crate::domain::GistComment;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;

/// Comments are fetched a page at a time, newest page first. 30 keeps each page small
/// for popular gists with hundreds–thousands of comments.
pub const COMMENTS_PAGE_SIZE: u32 = 30;

/// 1-based index of the last page holding `total` items at `per_page` each. Never 0, so
/// the caller always has a page to request (it skips the fetch entirely when total == 0).
pub fn last_page(total: u32, per_page: u32) -> u32 {
    if total == 0 {
        return 1;
    }
    total.div_ceil(per_page)
}

/// Everything in `raw` before the first blank line — the HTTP header block from `gh api -i`.
fn http_headers_section(raw: &str) -> &str {
    if let Some(i) = raw.find("\r\n\r\n") {
        &raw[..i]
    } else if let Some(i) = raw.find("\n\n") {
        &raw[..i]
    } else {
        raw
    }
}

/// Everything after the first blank line — the JSON body from `gh api -i`.
fn http_body_section(raw: &str) -> &str {
    if let Some(i) = raw.find("\r\n\r\n") {
        &raw[i + 4..]
    } else if let Some(i) = raw.find("\n\n") {
        &raw[i + 2..]
    } else {
        ""
    }
}

/// Read the `page=` number for a given `rel` ("next" / "last" / …) from an RFC 5988
/// `Link` header inside `gh api -i` output. Keys off `&page=` / `?page=` so the
/// `per_page=` parameter in the same URL is never mistaken for `page=`.
pub fn parse_link_rel(raw: &str, rel: &str) -> Option<u32> {
    let headers = http_headers_section(raw);
    let link = headers
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("link:"))?;
    let needle = format!("rel=\"{rel}\"");
    for part in link.split(',') {
        if !part.contains(&needle) {
            continue;
        }
        for marker in ["&page=", "?page="] {
            if let Some(idx) = part.find(marker) {
                let digits: String = part[idx + marker.len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(n) = digits.parse::<u32>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

/// Exact comment count from the `per_page=1` probe: the `Link` `rel="last"` page number
/// equals the total. With 0 or 1 comments there is no `Link` header, so fall back to the
/// item count in the probe body.
pub fn comments_total_from_probe(raw_i: &str) -> u32 {
    if let Some(last) = parse_link_rel(raw_i, "last") {
        return last;
    }
    parse_gist_comments_json(http_body_section(raw_i))
        .map(|v| v.len() as u32)
        .unwrap_or(0)
}

/// `gh api -i …per_page=1` — a tiny request whose `Link: rel="last"` reveals the total.
pub fn gist_comments_probe_plan(gist_id: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "-i".into(),
            format!("/gists/{gist_id}/comments?per_page=1"),
        ],
    }
}

/// Fetch exactly one page of comments (no `--paginate`, no headers — JSON body only).
pub fn gist_comments_page_plan(gist_id: &str, page: u32, per_page: u32) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            format!("/gists/{gist_id}/comments?per_page={per_page}&page={page}"),
        ],
    }
}

pub fn fetch_gist_comments_probe(runner: &dyn CommandRunner, gist_id: &str) -> Result<String> {
    run_command(runner, &gist_comments_probe_plan(gist_id))
}

pub fn fetch_gist_comments_page(
    runner: &dyn CommandRunner,
    gist_id: &str,
    page: u32,
    per_page: u32,
) -> Result<String> {
    run_command(runner, &gist_comments_page_plan(gist_id, page, per_page))
}

#[derive(Debug, Deserialize)]
struct GhComment {
    #[serde(default)]
    user: Option<GhCommentUser>,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    body: String,
}

pub fn parse_gist_comments_json(raw: &str) -> Result<Vec<GistComment>> {
    let comments: Vec<GhComment> =
        serde_json::from_str(raw).context("parse gh gist comments JSON")?;
    Ok(comments
        .into_iter()
        .map(|c| GistComment {
            author: c
                .user
                .map(|u| u.login)
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| "(unknown)".to_string()),
            created_at: c.created_at,
            body: c.body,
        })
        .collect())
}

/// Map each gist id to its comment count, parsed from the same gist-list JSON. The count rides
/// along in the list response, so this needs no extra `gh` call.
pub fn parse_gist_comment_counts(raw: &str) -> Result<HashMap<String, u32>> {
    let gists = parse_gh_gists(raw)?;
    Ok(gists.into_iter().map(|g| (g.id, g.comments)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comment_counts_defaulting_to_zero() {
        let raw = include_str!("../../tests/fixtures/gh/gist-list.json");
        let counts = parse_gist_comment_counts(raw).unwrap();

        assert_eq!(counts.get("abc123").copied(), Some(2));
        // The gist with no `comments` field falls back to 0 via `#[serde(default)]`.
        assert_eq!(counts.get("def456").copied(), Some(0));
    }

    #[test]
    fn last_page_is_ceiling_division() {
        assert_eq!(last_page(910, 30), 31); // 910/30 = 30.33 → 31
        assert_eq!(last_page(900, 30), 30); // exact
        assert_eq!(last_page(1, 30), 1);
        assert_eq!(last_page(0, 30), 1); // never zero — caller skips fetch when total==0
    }

    #[test]
    fn parse_link_rel_extracts_page_not_per_page() {
        // The trap: the URL has BOTH per_page=1 and page=910. Must return 910, not 1.
        let raw = "HTTP/2.0 200 OK\r\n\
Link: <https://api.github.com/gists/x/comments?per_page=1&page=2>; rel=\"next\", \
<https://api.github.com/gists/x/comments?per_page=1&page=910>; rel=\"last\"\r\n\
Content-Type: application/json\r\n\r\n[]";
        assert_eq!(parse_link_rel(raw, "last"), Some(910));
        assert_eq!(parse_link_rel(raw, "next"), Some(2));
        assert_eq!(parse_link_rel(raw, "prev"), None);
    }

    #[test]
    fn parse_link_rel_none_when_no_link_header() {
        let raw = "HTTP/2.0 200 OK\r\nContent-Type: application/json\r\n\r\n[]";
        assert_eq!(parse_link_rel(raw, "last"), None);
    }

    #[test]
    fn comments_total_from_probe_uses_rel_last() {
        let raw = "HTTP/2.0 200 OK\r\n\
Link: <https://api.github.com/gists/x/comments?per_page=1&page=910>; rel=\"last\"\r\n\r\n\
[{\"user\":{\"login\":\"a\"},\"created_at\":\"2026-01-01T00:00:00Z\",\"body\":\"hi\"}]";
        assert_eq!(comments_total_from_probe(raw), 910);
    }

    #[test]
    fn comments_total_from_probe_counts_body_when_single_page() {
        // 0 comments → empty body, no Link header → total 0.
        let zero = "HTTP/2.0 200 OK\r\nContent-Type: application/json\r\n\r\n[]";
        assert_eq!(comments_total_from_probe(zero), 0);
        // 1 comment → one item, no Link header → total 1.
        let one = "HTTP/2.0 200 OK\r\n\r\n\
[{\"user\":{\"login\":\"a\"},\"created_at\":\"2026-01-01T00:00:00Z\",\"body\":\"hi\"}]";
        assert_eq!(comments_total_from_probe(one), 1);
    }

    #[test]
    fn comments_page_plan_builds_single_page_request() {
        let plan = gist_comments_page_plan("abc", 31, 30);
        assert_eq!(plan.program, "gh");
        assert_eq!(
            plan.args,
            vec![
                "api".to_string(),
                "/gists/abc/comments?per_page=30&page=31".to_string(),
            ]
        );
    }

    #[test]
    fn comments_probe_plan_uses_include_and_per_page_1() {
        let plan = gist_comments_probe_plan("abc");
        assert_eq!(
            plan.args,
            vec![
                "api".to_string(),
                "-i".to_string(),
                "/gists/abc/comments?per_page=1".to_string(),
            ]
        );
    }
}
