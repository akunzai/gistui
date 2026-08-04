//! Gist fork counts, fork-flag detection, and fork_of resolution (issue #301).

use super::{escape_graphql_string, parse_gh_gists, GhGist};
use crate::actions::{run_command, CommandPlan, CommandRunner, SystemRunner};
use crate::domain::GistFile;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

/// Map each gist id to how many forks it has. Uses the `forks` array when the JSON
/// includes it (full gist); list responses omit it and return 0.
pub fn parse_gist_fork_counts(raw: &str) -> Result<HashMap<String, u32>> {
    let gists = parse_gh_gists(raw)?;
    Ok(gists
        .into_iter()
        .map(|g| (g.id, g.forks.len() as u32))
        .collect())
}

/// Plan for listing every fork of a gist (paginated; gh concatenates pages).
pub fn gist_forks_plan(gist_id: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "--paginate".into(),
            format!("/gists/{gist_id}/forks?per_page=100"),
        ],
    }
}

pub fn fetch_gist_fork_count_with(runner: &dyn CommandRunner, gist_id: &str) -> Result<u32> {
    let raw = run_command(runner, &gist_forks_plan(gist_id))?;
    let forks: Vec<serde_json::Value> = serde_json::from_str(&raw).context("parse gist forks")?;
    Ok(forks.len() as u32)
}

/// GraphQL query: the REST gist *list* omits `fork_of`, but `isFork` is reliable here.
/// One page of the viewer's gists with their `isFork` flag. Accounts with >100 gists need
/// pagination: `after` carries the previous page's `endCursor` (escaped, since it is opaque
/// API text), and the response's `pageInfo` drives the loop in
/// [`fetch_forked_gist_ids_graphql_with`].
fn gist_fork_flags_graphql_query(after: Option<&str>) -> String {
    let connection = match after {
        Some(cursor) => format!(
            "gists(first: 100, after: \"{}\")",
            escape_graphql_string(cursor)
        ),
        None => "gists(first: 100)".to_string(),
    };
    format!(
        "{{ viewer {{ {connection} {{ nodes {{ name isFork }} \
         pageInfo {{ hasNextPage endCursor }} }} }} }}"
    )
}

pub fn gist_fork_flags_graphql_plan(after: Option<&str>) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "graphql".into(),
            "-f".into(),
            format!("query={}", gist_fork_flags_graphql_query(after)),
        ],
    }
}

/// Plan for a single gist (`fork_of` is present on the full object, not the list).
pub fn gist_detail_plan(gist_id: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec!["api".into(), format!("/gists/{gist_id}")],
    }
}

pub fn fetch_forked_gist_ids_graphql_with(runner: &dyn CommandRunner) -> Result<HashSet<String>> {
    let mut all = HashSet::new();
    let mut after: Option<String> = None;
    loop {
        let raw = run_command(runner, &gist_fork_flags_graphql_plan(after.as_deref()))?;
        let (ids, next) = parse_fork_flags_page(&raw)?;
        all.extend(ids);
        match next {
            Some(cursor) => after = Some(cursor),
            None => break,
        }
    }
    Ok(all)
}

#[derive(Debug, Deserialize)]
struct GraphqlForkFlagsResponse {
    data: GraphqlForkFlagsData,
}

#[derive(Debug, Deserialize)]
struct GraphqlForkFlagsData {
    viewer: GraphqlForkFlagsViewer,
}

#[derive(Debug, Deserialize)]
struct GraphqlForkFlagsViewer {
    gists: GraphqlForkFlagsConnection,
}

#[derive(Debug, Deserialize)]
struct GraphqlForkFlagsConnection {
    nodes: Vec<GraphqlForkFlagsNode>,
    /// Absent in older single-page fixtures, so default to a terminal page.
    #[serde(rename = "pageInfo", default)]
    page_info: GraphqlPageInfo,
}

#[derive(Debug, Deserialize, Default)]
struct GraphqlPageInfo {
    #[serde(rename = "hasNextPage", default)]
    has_next_page: bool,
    #[serde(rename = "endCursor", default)]
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphqlForkFlagsNode {
    /// Gist id (hex), not the filename.
    name: String,
    #[serde(rename = "isFork")]
    is_fork: bool,
}

/// Parse one fork-flags page into its fork ids plus the cursor for the next page (`None`
/// when `hasNextPage` is false).
fn parse_fork_flags_page(raw: &str) -> Result<(HashSet<String>, Option<String>)> {
    let resp: GraphqlForkFlagsResponse =
        serde_json::from_str(raw).context("parse gist fork flags GraphQL")?;
    let connection = resp.data.viewer.gists;
    let ids = connection
        .nodes
        .into_iter()
        .filter(|n| n.is_fork)
        .map(|n| n.name)
        .collect();
    let next = connection
        .page_info
        .has_next_page
        .then_some(connection.page_info.end_cursor)
        .flatten();
    Ok((ids, next))
}

/// Owned gist ids flagged as forks by a single GraphQL viewer page (the cursor is dropped).
/// Pagination is handled by [`fetch_forked_gist_ids_graphql_with`].
pub fn parse_forked_gist_ids_graphql(raw: &str) -> Result<HashSet<String>> {
    parse_fork_flags_page(raw).map(|(ids, _)| ids)
}

pub fn fetch_gist_fork_of_id_with(
    runner: &dyn CommandRunner,
    gist_id: &str,
) -> Result<Option<String>> {
    let raw = run_command(runner, &gist_detail_plan(gist_id))?;
    let gist: GhGist = serde_json::from_str(&raw).context("parse gh gist detail JSON")?;
    Ok(gist.fork_of.map(|f| f.id))
}

/// Map owned gist id → upstream `fork_of` id. Uses GraphQL `isFork` (one call) then
/// `GET /gists/{id}` only for the handful of owned forks (list JSON omits `fork_of`).
pub fn collect_owned_fork_of_ids(
    owned_ids: HashSet<String>,
) -> Result<HashMap<String, Option<String>>, String> {
    collect_owned_fork_of_ids_with(&SystemRunner, owned_ids)
}

/// Injectable variant of [`collect_owned_fork_of_ids`] (issue #245).
pub fn collect_owned_fork_of_ids_with(
    runner: &dyn CommandRunner,
    owned_ids: HashSet<String>,
) -> Result<HashMap<String, Option<String>>, String> {
    // Surface a failure of the single fork-detection query — a transient error or expired
    // token would otherwise leave every owned fork undetected with no hint why the `forked`
    // filter is empty. Per-gist `fork_of` lookups stay best-effort (one bad gist is skipped).
    let fork_ids = fetch_forked_gist_ids_graphql_with(runner).map_err(|e| e.to_string())?;
    let mut out = HashMap::new();
    for id in fork_ids.intersection(&owned_ids) {
        if let Ok(fork_of) = fetch_gist_fork_of_id_with(runner, id) {
            out.insert(id.clone(), fork_of);
        }
    }
    Ok(out)
}

/// Stamp `fork_of_id` onto every [`GistFile`] row for gists present in `fork_of`.
pub fn apply_fork_of_ids(gists: &mut [GistFile], fork_of: &HashMap<String, Option<String>>) {
    for g in gists.iter_mut() {
        if let Some(upstream) = fork_of.get(&g.gist_id) {
            g.fork_of_id = upstream.clone();
        }
    }
}

/// Fill fork counts. List JSON usually omits `forks`, so each id is probed via
/// `/gists/{id}/forks` when the parsed count is zero. Merges owned and starred list JSON.
pub fn collect_gist_fork_counts(
    owned_raw: Option<&str>,
    starred_raw: Option<&str>,
    gist_ids: impl IntoIterator<Item = String>,
) -> HashMap<String, u32> {
    collect_gist_fork_counts_with(&SystemRunner, owned_raw, starred_raw, gist_ids)
}

/// Injectable variant of [`collect_gist_fork_counts`] (issue #245).
pub fn collect_gist_fork_counts_with(
    runner: &dyn CommandRunner,
    owned_raw: Option<&str>,
    starred_raw: Option<&str>,
    gist_ids: impl IntoIterator<Item = String>,
) -> HashMap<String, u32> {
    let mut counts = owned_raw
        .and_then(|raw| parse_gist_fork_counts(raw).ok())
        .unwrap_or_default();
    if let Some(raw) = starred_raw {
        if let Ok(starred) = parse_gist_fork_counts(raw) {
            counts.extend(starred);
        }
    }
    for id in gist_ids {
        if counts.get(&id).copied().unwrap_or(0) > 0 {
            continue;
        }
        if let Ok(n) = fetch_gist_fork_count_with(runner, &id) {
            if n > 0 {
                counts.insert(id, n);
            }
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fork_counts_from_forks_array() {
        let raw = r#"[{"id":"a","comments":0,"forks":[{},{}]},{"id":"b","comments":0}]"#;
        let counts = parse_gist_fork_counts(raw).unwrap();
        assert_eq!(counts.get("a").copied(), Some(2));
        assert_eq!(counts.get("b").copied(), Some(0));
    }

    #[test]
    fn parses_forked_gist_ids_from_graphql() {
        let raw = r#"{
            "data": {
                "viewer": {
                    "gists": {
                        "nodes": [
                            {"name": "owned1", "isFork": false},
                            {"name": "fork1", "isFork": true},
                            {"name": "fork2", "isFork": true}
                        ]
                    }
                }
            }
        }"#;
        let ids = parse_forked_gist_ids_graphql(raw).unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("fork1"));
        assert!(ids.contains("fork2"));
    }

    #[test]
    fn fork_flags_page_reports_next_cursor_then_stops() {
        // A page with hasNextPage drives the pagination loop forward via its endCursor.
        let more = r#"{ "data": { "viewer": { "gists": {
            "nodes": [{"name": "fork1", "isFork": true}, {"name": "owned1", "isFork": false}],
            "pageInfo": {"hasNextPage": true, "endCursor": "CUR2"}
        } } } }"#;
        let (ids, next) = parse_fork_flags_page(more).unwrap();
        assert_eq!(
            ids.into_iter().collect::<Vec<_>>(),
            vec!["fork1".to_string()]
        );
        assert_eq!(next, Some("CUR2".to_string()));

        // The last page has no next cursor — the loop terminates.
        let last = r#"{ "data": { "viewer": { "gists": {
            "nodes": [{"name": "fork2", "isFork": true}],
            "pageInfo": {"hasNextPage": false, "endCursor": "CUR9"}
        } } } }"#;
        let (_, next) = parse_fork_flags_page(last).unwrap();
        assert_eq!(next, None);
    }

    #[test]
    fn fork_flags_query_escapes_cursor_and_omits_after_on_first_page() {
        assert!(!gist_fork_flags_graphql_query(None).contains("after"));
        let q = gist_fork_flags_graphql_query(Some("a\"b"));
        assert!(q.contains(r#"after: "a\"b""#));
    }

    #[test]
    fn apply_fork_of_ids_stamps_all_file_rows() {
        let mut gists = vec![
            GistFile {
                gist_id: "fork1".into(),
                description: String::new(),
                filename: "a.txt".into(),
                public: true,
                updated_at: String::new(),
                created_at: String::new(),
                owner_login: "me".into(),
                fork_of_id: None,
                raw_url: None,
                content_type: None,
                node_id: None,
            },
            GistFile {
                gist_id: "fork1".into(),
                description: String::new(),
                filename: "b.txt".into(),
                public: true,
                updated_at: String::new(),
                created_at: String::new(),
                owner_login: "me".into(),
                fork_of_id: None,
                raw_url: None,
                content_type: None,
                node_id: None,
            },
        ];
        let fork_of = [("fork1".into(), Some("upstream".into()))].into();
        apply_fork_of_ids(&mut gists, &fork_of);
        assert!(gists
            .iter()
            .all(|g| g.fork_of_id.as_deref() == Some("upstream")));
    }

    #[test]
    fn collect_gist_fork_counts_with_probes_zero_entries() {
        use crate::actions::test_support::SeqRunner;
        use crate::actions::CommandOutput;

        // List raw has no per-id fork arrays → every id is probed via /forks.
        let list_raw =
            r#"[{"id":"abc123","files":{},"comments":0},{"id":"def456","files":{},"comments":0}]"#;
        let runner = SeqRunner::new(vec![
            CommandOutput {
                success: true,
                stdout: r#"[{"id":"f1"},{"id":"f2"}]"#.into(),
                stderr: String::new(),
            },
            CommandOutput {
                success: true,
                stdout: "[]".into(),
                stderr: String::new(),
            },
        ]);
        let counts = collect_gist_fork_counts_with(
            &runner,
            Some(list_raw),
            None,
            ["abc123".into(), "def456".into()],
        );
        assert_eq!(counts.get("abc123").copied(), Some(2));
        // Probe returned empty forks → count stays 0 (from list parse or absent).
        assert_eq!(counts.get("def456").copied().unwrap_or(0), 0);
        let calls = runner.calls();
        assert_eq!(calls[0], gist_forks_plan("abc123"));
        assert_eq!(calls[1], gist_forks_plan("def456"));
    }

    #[test]
    fn collect_owned_fork_of_ids_with_maps_owned_forks() {
        use crate::actions::test_support::SeqRunner;
        use crate::actions::CommandOutput;

        // GraphQL: g1 is a fork; g2 is not. Then GET /gists/g1 for fork_of.
        let graphql = r#"{
            "data": {
                "viewer": {
                    "gists": {
                        "nodes": [
                            {"name": "g1", "isFork": true},
                            {"name": "g2", "isFork": false}
                        ],
                        "pageInfo": {"hasNextPage": false, "endCursor": null}
                    }
                }
            }
        }"#;
        let detail = r#"{"id":"g1","description":"","public":false,"updated_at":"","created_at":"","files":{},"fork_of":{"id":"upstream1"}}"#;
        let runner = SeqRunner::new(vec![
            CommandOutput {
                success: true,
                stdout: graphql.into(),
                stderr: String::new(),
            },
            CommandOutput {
                success: true,
                stdout: detail.into(),
                stderr: String::new(),
            },
        ]);
        let owned = HashSet::from(["g1".into(), "g2".into(), "g3".into()]);
        let map = collect_owned_fork_of_ids_with(&runner, owned).unwrap();
        assert_eq!(map.get("g1").cloned(), Some(Some("upstream1".into())));
        assert!(!map.contains_key("g2"));
        assert!(!map.contains_key("g3"));
        let calls = runner.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1], gist_detail_plan("g1"));
    }

    #[test]
    fn collect_owned_fork_of_ids_with_surfaces_graphql_failure() {
        use crate::actions::test_support::SeqRunner;
        use crate::actions::CommandOutput;

        let runner = SeqRunner::new(vec![CommandOutput {
            success: false,
            stdout: String::new(),
            stderr: "HTTP 401".into(),
        }]);
        let err =
            collect_owned_fork_of_ids_with(&runner, HashSet::from(["g1".into()])).unwrap_err();
        assert!(err.contains("401") || err.contains("HTTP"));
    }
}
