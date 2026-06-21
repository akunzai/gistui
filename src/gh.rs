use crate::actions::{run_command, CommandPlan, CommandRunner, SystemRunner};
use crate::domain::{GistComment, GistFile, GistRevision, GistRevisionChangeStatus};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Deserialize)]
struct GhGist {
    id: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    public: bool,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    created_at: String,
    /// Number of comments on the gist. The REST list response includes this, so the count is
    /// available without a per-gist comments fetch.
    #[serde(default)]
    comments: u32,
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    owner: Option<GhCommentUser>,
    #[serde(default)]
    fork_of: Option<GhGistForkOf>,
    /// Present on full gist objects; omitted from the list response (counts default to 0).
    #[serde(default)]
    forks: Vec<serde_json::Value>,
    // The REST API returns `files` as an object keyed by filename. BTreeMap keeps
    // the order deterministic (by filename) for stable display and tests.
    #[serde(default)]
    files: BTreeMap<String, GhGistFile>,
}

#[derive(Debug, Deserialize)]
struct GhGistForkOf {
    id: String,
}

#[derive(Debug, Deserialize)]
struct GhGistFile {
    filename: String,
    #[serde(default)]
    raw_url: Option<String>,
    #[serde(default, rename = "type")]
    content_type: Option<String>,
}

/// Plan for `gh --version` (used to confirm `gh` is installed and runnable).
pub fn gh_version_plan() -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec!["--version".into()],
    }
}

/// Plan for `gh auth status` (used to confirm an authenticated session).
pub fn auth_status_plan() -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec!["auth".into(), "status".into()],
    }
}

/// Plan for listing every gist via the REST API.
///
/// `gh gist list` has no `--json` flag; use the REST API with `--paginate` so
/// accounts with more than 100 gists are fully retrieved. gh concatenates all
/// pages into a single JSON array, which `parse_gist_list_json` already handles.
pub fn gist_list_plan() -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "--paginate".into(),
            "/gists?per_page=100".into(),
        ],
    }
}

/// Plan for listing the authenticated user's starred gists.
pub fn gist_starred_list_plan() -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "--paginate".into(),
            "/gists/starred?per_page=100".into(),
        ],
    }
}

/// Plan for the authenticated user's login (ownership checks).
pub fn current_user_plan() -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec!["api".into(), "user".into(), "--jq".into(), ".login".into()],
    }
}

/// Plan for fetching a single gist file's raw content.
pub fn gist_view_plan(gist_id: &str, filename: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "gist".into(),
            "view".into(),
            gist_id.to_string(),
            "--filename".into(),
            filename.to_string(),
            "--raw".into(),
        ],
    }
}

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

/// Plan for fetching all gist comments via the REST API (paginated).
/// Kept for backward compatibility; will be removed when run_loop is rewired in a later task.
#[allow(dead_code)]
pub fn gist_comments_plan(gist_id: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "--paginate".into(),
            format!("/gists/{gist_id}/comments?per_page=100"),
        ],
    }
}

#[allow(dead_code)]
pub fn fetch_gist_comments_json(gist_id: &str) -> Result<String> {
    fetch_gist_comments_json_with(&SystemRunner, gist_id)
}

#[allow(dead_code)]
pub fn fetch_gist_comments_json_with(runner: &dyn CommandRunner, gist_id: &str) -> Result<String> {
    run_command(runner, &gist_comments_plan(gist_id))
}

pub fn fetch_gist_comments_probe(gist_id: &str) -> Result<String> {
    fetch_gist_comments_probe_with(&SystemRunner, gist_id)
}

pub fn fetch_gist_comments_probe_with(runner: &dyn CommandRunner, gist_id: &str) -> Result<String> {
    run_command(runner, &gist_comments_probe_plan(gist_id))
}

pub fn fetch_gist_comments_page(gist_id: &str, page: u32, per_page: u32) -> Result<String> {
    fetch_gist_comments_page_with(&SystemRunner, gist_id, page, per_page)
}

pub fn fetch_gist_comments_page_with(
    runner: &dyn CommandRunner,
    gist_id: &str,
    page: u32,
    per_page: u32,
) -> Result<String> {
    run_command(runner, &gist_comments_page_plan(gist_id, page, per_page))
}

pub fn check_gh_ready() -> Result<()> {
    check_gh_ready_with(&SystemRunner)
}

pub fn check_gh_ready_with(runner: &dyn CommandRunner) -> Result<()> {
    if !runner.run(&gh_version_plan())?.success {
        bail!("gh is installed but did not run successfully");
    }
    if !runner.run(&auth_status_plan())?.success {
        bail!("gh auth status failed; run gh auth login");
    }
    Ok(())
}

pub fn parse_gist_list_json(raw: &str) -> Result<Vec<GistFile>> {
    let gists: Vec<GhGist> = serde_json::from_str(raw).context("parse gh gist list JSON")?;
    let mut files = Vec::new();

    for gist in gists {
        let description = gist.description.unwrap_or_default();
        let owner_login = gist
            .owner
            .map(|u| u.login)
            .filter(|l| !l.is_empty())
            .unwrap_or_default();
        let fork_of_id = gist.fork_of.map(|f| f.id);
        for file in gist.files.into_values() {
            files.push(GistFile {
                gist_id: gist.id.clone(),
                description: description.clone(),
                filename: file.filename,
                public: gist.public,
                updated_at: gist.updated_at.clone(),
                created_at: gist.created_at.clone(),
                owner_login: owner_login.clone(),
                fork_of_id: fork_of_id.clone(),
                raw_url: file.raw_url.clone(),
                content_type: file.content_type.clone(),
                node_id: gist.node_id.clone(),
            });
        }
    }

    Ok(files)
}

/// Unique `gist_id → node_id` pairs from flat gist rows (first wins).
pub fn gist_node_id_map(files: &[GistFile]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for file in files {
        if let Some(nid) = file.node_id.as_ref().filter(|s| !s.is_empty()) {
            map.entry(file.gist_id.clone())
                .or_insert_with(|| nid.clone());
        }
    }
    map
}

/// Merge node-id maps from owned and starred gist rows.
pub fn merge_gist_node_id_maps(
    owned: &[GistFile],
    starred: &[GistFile],
) -> HashMap<String, String> {
    let mut map = gist_node_id_map(owned);
    for (id, nid) in gist_node_id_map(starred) {
        map.entry(id).or_insert(nid);
    }
    map
}

/// Map each gist id to its comment count, parsed from the same gist-list JSON. The count rides
/// along in the list response, so this needs no extra `gh` call.
pub fn parse_gist_comment_counts(raw: &str) -> Result<HashMap<String, u32>> {
    let gists: Vec<GhGist> = serde_json::from_str(raw).context("parse gh gist list JSON")?;
    Ok(gists.into_iter().map(|g| (g.id, g.comments)).collect())
}

/// Map each gist id to how many forks it has. Uses the `forks` array when the JSON
/// includes it (full gist); list responses omit it and return 0.
pub fn parse_gist_fork_counts(raw: &str) -> Result<HashMap<String, u32>> {
    let gists: Vec<GhGist> = serde_json::from_str(raw).context("parse gh gist list JSON")?;
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

pub fn fetch_gist_fork_count(gist_id: &str) -> Result<u32> {
    fetch_gist_fork_count_with(&SystemRunner, gist_id)
}

pub fn fetch_gist_fork_count_with(runner: &dyn CommandRunner, gist_id: &str) -> Result<u32> {
    let raw = run_command(runner, &gist_forks_plan(gist_id))?;
    let forks: Vec<serde_json::Value> = serde_json::from_str(&raw).context("parse gist forks")?;
    Ok(forks.len() as u32)
}

/// GraphQL query: the REST gist *list* omits `fork_of`, but `isFork` is reliable here.
const GIST_FORK_FLAGS_QUERY: &str = "{ viewer { gists(first: 100) { nodes { name isFork } } } }";

pub fn gist_fork_flags_graphql_plan() -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "graphql".into(),
            "-f".into(),
            format!("query={GIST_FORK_FLAGS_QUERY}"),
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

pub fn fetch_forked_gist_ids_graphql() -> Result<HashSet<String>> {
    fetch_forked_gist_ids_graphql_with(&SystemRunner)
}

pub fn fetch_forked_gist_ids_graphql_with(runner: &dyn CommandRunner) -> Result<HashSet<String>> {
    let raw = run_command(runner, &gist_fork_flags_graphql_plan())?;
    parse_forked_gist_ids_graphql(&raw)
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
}

#[derive(Debug, Deserialize)]
struct GraphqlForkFlagsNode {
    /// Gist id (hex), not the filename.
    name: String,
    #[serde(rename = "isFork")]
    is_fork: bool,
}

/// Owned gist ids flagged as forks by the GraphQL viewer query.
pub fn parse_forked_gist_ids_graphql(raw: &str) -> Result<HashSet<String>> {
    let resp: GraphqlForkFlagsResponse =
        serde_json::from_str(raw).context("parse gist fork flags GraphQL")?;
    Ok(resp
        .data
        .viewer
        .gists
        .nodes
        .into_iter()
        .filter(|n| n.is_fork)
        .map(|n| n.name)
        .collect())
}

pub fn fetch_gist_fork_of_id(gist_id: &str) -> Result<Option<String>> {
    fetch_gist_fork_of_id_with(&SystemRunner, gist_id)
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
pub fn collect_owned_fork_of_ids(owned_ids: HashSet<String>) -> HashMap<String, Option<String>> {
    let fork_ids = match fetch_forked_gist_ids_graphql() {
        Ok(ids) => ids,
        Err(_) => return HashMap::new(),
    };
    let mut out = HashMap::new();
    for id in fork_ids.intersection(&owned_ids) {
        if let Ok(fork_of) = fetch_gist_fork_of_id(id) {
            out.insert(id.clone(), fork_of);
        }
    }
    out
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
        if let Ok(n) = fetch_gist_fork_count(&id) {
            if n > 0 {
                counts.insert(id, n);
            }
        }
    }
    counts
}

const STARGAZER_GRAPHQL_CHUNK: usize = 40;

/// Build a batched GraphQL query (`n0`…`n{k}` aliases) for stargazer counts.
pub fn build_stargazer_graphql_query(node_ids: &[String]) -> String {
    let mut query = String::from("query { ");
    for (i, id) in node_ids.iter().enumerate() {
        query.push_str(&format!(
            "n{i}: node(id: \"{id}\") {{ ... on Gist {{ name stargazerCount }} }} "
        ));
    }
    query.push('}');
    query
}

pub fn gist_stargazer_graphql_plan(query: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "graphql".into(),
            "-f".into(),
            format!("query={query}"),
        ],
    }
}

/// Parse alias-keyed GraphQL data (`n0`, `n1`, …) into `gist_id → stargazerCount`.
pub fn parse_stargazer_counts_graphql(raw: &str) -> Result<HashMap<String, u32>> {
    let v: serde_json::Value = serde_json::from_str(raw).context("parse stargazer GraphQL")?;
    let data = v
        .get("data")
        .and_then(|d| d.as_object())
        .context("GraphQL data object")?;
    let mut out = HashMap::new();
    for node in data.values() {
        if node.is_null() {
            continue;
        }
        let Some(name) = node.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        let count = node
            .get("stargazerCount")
            .and_then(|n| n.as_u64())
            .unwrap_or(0) as u32;
        if count > 0 {
            out.insert(name.to_string(), count);
        }
    }
    Ok(out)
}

pub fn collect_gist_star_counts(node_ids: HashMap<String, String>) -> HashMap<String, u32> {
    collect_gist_star_counts_with(&SystemRunner, node_ids)
}

pub fn collect_gist_star_counts_with(
    runner: &dyn CommandRunner,
    node_ids: HashMap<String, String>,
) -> HashMap<String, u32> {
    let ids: Vec<String> = node_ids.into_values().collect();
    let mut out = HashMap::new();
    for chunk in ids.chunks(STARGAZER_GRAPHQL_CHUNK) {
        let query = build_stargazer_graphql_query(chunk);
        let raw = match run_command(runner, &gist_stargazer_graphql_plan(&query)) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        if let Ok(batch) = parse_stargazer_counts_graphql(&raw) {
            out.extend(batch);
        }
    }
    out
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

#[derive(Debug, Deserialize)]
struct GhCommentUser {
    #[serde(default)]
    login: String,
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

pub fn fetch_gist_list_json() -> Result<String> {
    fetch_gist_list_json_with(&SystemRunner)
}

pub fn fetch_gist_list_json_with(runner: &dyn CommandRunner) -> Result<String> {
    run_command(runner, &gist_list_plan())
}

pub fn fetch_gist_starred_list_json() -> Result<String> {
    fetch_gist_starred_list_json_with(&SystemRunner)
}

pub fn fetch_gist_starred_list_json_with(runner: &dyn CommandRunner) -> Result<String> {
    run_command(runner, &gist_starred_list_plan())
}

pub fn fetch_current_user_login() -> Result<String> {
    fetch_current_user_login_with(&SystemRunner)
}

pub fn fetch_current_user_login_with(runner: &dyn CommandRunner) -> Result<String> {
    let raw = run_command(runner, &current_user_plan())?;
    let login = raw.trim().trim_matches('"').to_string();
    if login.is_empty() {
        anyhow::bail!("empty user login from gh api user");
    }
    Ok(login)
}

/// Unique gist ids from a parsed gist-list JSON payload.
pub fn parse_starred_gist_ids(raw: &str) -> Result<std::collections::HashSet<String>> {
    let gists: Vec<GhGist> = serde_json::from_str(raw).context("parse gh gist list JSON")?;
    Ok(gists.into_iter().map(|g| g.id).collect())
}

/// Plan for fetching gist file bytes from a list-response `raw_url` (no auth).
pub fn raw_url_fetch_plan(url: &str) -> CommandPlan {
    CommandPlan {
        program: "curl".into(),
        args: vec!["-sL".into(), url.into()],
    }
}

pub fn fetch_gist_file_content(
    gist_id: &str,
    filename: &str,
    raw_url: Option<&str>,
) -> Result<String> {
    fetch_gist_file_content_with(&SystemRunner, gist_id, filename, raw_url)
}

pub fn fetch_gist_file_content_with(
    runner: &dyn CommandRunner,
    gist_id: &str,
    filename: &str,
    raw_url: Option<&str>,
) -> Result<String> {
    match run_command(runner, &gist_view_plan(gist_id, filename)) {
        Ok(content) => Ok(content),
        Err(primary) => {
            if let Some(url) = raw_url.filter(|u| !u.is_empty()) {
                run_command(runner, &raw_url_fetch_plan(url))
                    .with_context(|| format!("{primary}; raw_url fallback also failed"))
            } else {
                Err(primary)
            }
        }
    }
}

/// Plan for listing every revision of a gist via the REST API.
pub fn gist_commits_plan(gist_id: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec![
            "api".into(),
            "--paginate".into(),
            format!("/gists/{gist_id}/commits?per_page=100"),
        ],
    }
}

/// Plan for fetching a single gist revision snapshot (files + metadata at that SHA).
pub fn gist_revision_plan(gist_id: &str, version: &str) -> CommandPlan {
    CommandPlan {
        program: "gh".into(),
        args: vec!["api".into(), format!("/gists/{gist_id}/{version}")],
    }
}

pub fn fetch_gist_commits_json(gist_id: &str) -> Result<String> {
    fetch_gist_commits_json_with(&SystemRunner, gist_id)
}

pub fn fetch_gist_commits_json_with(runner: &dyn CommandRunner, gist_id: &str) -> Result<String> {
    run_command(runner, &gist_commits_plan(gist_id))
}

pub fn fetch_gist_revision_json(gist_id: &str, version: &str) -> Result<String> {
    fetch_gist_revision_json_with(&SystemRunner, gist_id, version)
}

pub fn fetch_gist_revision_json_with(
    runner: &dyn CommandRunner,
    gist_id: &str,
    version: &str,
) -> Result<String> {
    run_command(runner, &gist_revision_plan(gist_id, version))
}

/// Canonical gist revision raw URL (`owner` form works for large third-party gists).
pub fn build_gist_revision_raw_url(
    owner_login: &str,
    gist_id: &str,
    version: &str,
    filename: &str,
) -> String {
    if owner_login.is_empty() {
        format!("https://gist.githubusercontent.com/{gist_id}/raw/{version}/{filename}")
    } else {
        format!(
            "https://gist.githubusercontent.com/{owner_login}/{gist_id}/raw/{version}/{filename}"
        )
    }
}

fn revision_file_entry<'a>(
    files: &'a serde_json::Map<String, serde_json::Value>,
    filename: &str,
) -> Option<&'a serde_json::Value> {
    if let Some(entry) = files.get(filename) {
        return Some(entry);
    }
    files
        .values()
        .find(|entry| entry.get("filename").and_then(|f| f.as_str()) == Some(filename))
}

fn revision_entry_raw_url(entry: &serde_json::Value) -> Option<String> {
    entry
        .get("raw_url")
        .and_then(|u| u.as_str())
        .filter(|u| !u.is_empty())
        .map(str::to_string)
}

fn fetch_revision_file_via_raw_url(
    runner: &dyn CommandRunner,
    url: &str,
) -> Result<RevisionFileContent> {
    run_command(runner, &raw_url_fetch_plan(url)).map(RevisionFileContent::Present)
}

/// Fetch one file at a gist revision SHA. Uses the revision API when it works; on HTTP
/// failures or truncated payloads, falls back to the revision `raw_url` or the canonical
/// `gist.githubusercontent.com/.../raw/{sha}/{file}` URL.
pub fn fetch_revision_file_with(
    runner: &dyn CommandRunner,
    gist_id: &str,
    version: &str,
    filename: &str,
    owner_login: &str,
) -> Result<RevisionFileContent> {
    let constructed = build_gist_revision_raw_url(owner_login, gist_id, version, filename);
    match fetch_gist_revision_json_with(runner, gist_id, version) {
        Ok(raw) => {
            let root: serde_json::Value =
                serde_json::from_str(&raw).context("parse gh gist revision JSON")?;
            let Some(files) = root.get("files").and_then(|f| f.as_object()) else {
                return Ok(RevisionFileContent::Absent);
            };
            let Some(entry) = revision_file_entry(files, filename) else {
                return Ok(RevisionFileContent::Absent);
            };
            match classify_revision_file(entry)? {
                RevisionFileContent::Present(content) => Ok(RevisionFileContent::Present(content)),
                RevisionFileContent::Truncated => revision_entry_raw_url(entry)
                    .map(|url| fetch_revision_file_via_raw_url(runner, &url))
                    .unwrap_or_else(|| fetch_revision_file_via_raw_url(runner, &constructed)),
                RevisionFileContent::Absent => revision_entry_raw_url(entry)
                    .map(|url| fetch_revision_file_via_raw_url(runner, &url))
                    .unwrap_or(Ok(RevisionFileContent::Absent)),
            }
        }
        Err(api_err) => fetch_revision_file_via_raw_url(runner, &constructed).with_context(|| {
            format!("revision API failed ({api_err}); raw URL fallback also failed")
        }),
    }
}

pub fn fetch_revision_file_text(
    gist_id: &str,
    version: &str,
    filename: &str,
    owner_login: &str,
) -> Result<String> {
    fetch_revision_file_text_with(&SystemRunner, gist_id, version, filename, owner_login)
}

pub fn fetch_revision_file_text_with(
    runner: &dyn CommandRunner,
    gist_id: &str,
    version: &str,
    filename: &str,
    owner_login: &str,
) -> Result<String> {
    match fetch_revision_file_with(runner, gist_id, version, filename, owner_login)? {
        RevisionFileContent::Present(content) => Ok(content),
        RevisionFileContent::Truncated => {
            bail!("file too large for API preview (>1 MB)")
        }
        RevisionFileContent::Absent => bail!("{filename} not present in this revision"),
    }
}

pub fn fetch_revision_file_text_optional(
    gist_id: &str,
    version: &str,
    filename: &str,
    owner_login: &str,
) -> Result<String> {
    fetch_revision_file_text_optional_with(&SystemRunner, gist_id, version, filename, owner_login)
}

pub fn fetch_revision_file_text_optional_with(
    runner: &dyn CommandRunner,
    gist_id: &str,
    version: &str,
    filename: &str,
    owner_login: &str,
) -> Result<String> {
    match fetch_revision_file_with(runner, gist_id, version, filename, owner_login)? {
        RevisionFileContent::Present(content) => Ok(content),
        RevisionFileContent::Truncated => bail!("file too large for API preview (>1 MB)"),
        RevisionFileContent::Absent => Ok(String::new()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionFileContent {
    Present(String),
    Truncated,
    Absent,
}

#[derive(Debug, Deserialize)]
struct GhGistCommit {
    version: String,
    #[serde(default)]
    committed_at: String,
    #[serde(default)]
    user: Option<GhCommentUser>,
    #[serde(default)]
    change_status: GhGistChangeStatus,
}

#[derive(Debug, Deserialize, Default)]
struct GhGistChangeStatus {
    #[serde(default)]
    total: u32,
    #[serde(default)]
    additions: u32,
    #[serde(default)]
    deletions: u32,
}

pub fn parse_gist_commits_json(raw: &str) -> Result<Vec<GistRevision>> {
    let commits: Vec<GhGistCommit> =
        serde_json::from_str(raw).context("parse gh gist commits JSON")?;
    Ok(commits
        .into_iter()
        .map(|c| GistRevision {
            version: c.version,
            committed_at: c.committed_at,
            user: c
                .user
                .map(|u| u.login)
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| "(unknown)".to_string()),
            change_status: GistRevisionChangeStatus {
                total: c.change_status.total,
                additions: c.change_status.additions,
                deletions: c.change_status.deletions,
            },
        })
        .collect())
}

/// Extract one file's text from a revision snapshot (`GET /gists/{id}/{sha}`).
pub fn revision_file_content(raw: &str, filename: &str) -> Result<RevisionFileContent> {
    let root: serde_json::Value =
        serde_json::from_str(raw).context("parse gh gist revision JSON")?;
    let Some(files) = root.get("files").and_then(|f| f.as_object()) else {
        return Ok(RevisionFileContent::Absent);
    };
    if let Some(entry) = files.get(filename) {
        return classify_revision_file(entry);
    }
    for entry in files.values() {
        if entry.get("filename").and_then(|f| f.as_str()) == Some(filename) {
            return classify_revision_file(entry);
        }
    }
    Ok(RevisionFileContent::Absent)
}

fn classify_revision_file(entry: &serde_json::Value) -> Result<RevisionFileContent> {
    if entry.get("truncated").and_then(|t| t.as_bool()) == Some(true) {
        return Ok(RevisionFileContent::Truncated);
    }
    match entry.get("content").and_then(|c| c.as_str()) {
        Some(content) => Ok(RevisionFileContent::Present(content.to_string())),
        None => Ok(RevisionFileContent::Absent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gist_list_into_file_rows() {
        let raw = include_str!("../tests/fixtures/gh/gist-list.json");
        let files = parse_gist_list_json(raw).unwrap();

        assert_eq!(files.len(), 3);
        // Files within a gist are ordered deterministically by filename.
        assert_eq!(files[0].gist_id, "abc123");
        assert_eq!(files[0].filename, "settings.json");
        assert_eq!(files[0].description, "claude config");
        assert!(!files[0].public);
        assert_eq!(files[0].owner_login, "akunzai");
        assert_eq!(files[0].content_type.as_deref(), Some("application/json"));
        assert_eq!(files[1].filename, "statusline.sh");
        assert_eq!(files[1].content_type.as_deref(), Some("text/x-shellscript"));
        let notes = files.iter().find(|f| f.filename == "notes.md").unwrap();
        assert_eq!(notes.fork_of_id.as_deref(), Some("upstream99"));
    }

    #[test]
    fn parses_starred_gist_ids() {
        let raw = include_str!("../tests/fixtures/gh/gist-starred.json");
        let ids = parse_starred_gist_ids(raw).unwrap();
        assert_eq!(ids.len(), 1);
        assert!(ids.contains("star111"));
        let files = parse_gist_list_json(raw).unwrap();
        assert_eq!(files[0].owner_login, "otherdev");
    }

    #[test]
    fn null_description_parses_as_empty_string() {
        let raw = include_str!("../tests/fixtures/gh/gist-list.json");
        let files = parse_gist_list_json(raw).unwrap();

        let notes = files.iter().find(|f| f.filename == "notes.md").unwrap();
        assert_eq!(notes.description, "");
        assert!(notes.public);
    }

    #[test]
    fn parses_gist_commits_into_revisions() {
        let raw = include_str!("../tests/fixtures/gh/gist-commits.json");
        let revisions = parse_gist_commits_json(raw).unwrap();
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].version, "abc111def222333444");
        assert_eq!(revisions[0].user, "akunzai");
        assert_eq!(revisions[0].change_status.additions, 2);
        assert_eq!(revisions[0].change_status.deletions, 1);
        assert_eq!(revisions[1].committed_at, "2026-06-01T08:00:00Z");
    }

    #[test]
    fn revision_file_content_reads_present_and_truncated() {
        let raw = include_str!("../tests/fixtures/gh/gist-revision.json");
        match revision_file_content(raw, "settings.json").unwrap() {
            RevisionFileContent::Present(content) => {
                assert!(content.contains("\"old\": true"));
            }
            other => panic!("expected Present, got {other:?}"),
        }
        let truncated = r#"{"files":{"a.txt":{"filename":"a.txt","truncated":true}}}"#;
        assert_eq!(
            revision_file_content(truncated, "a.txt").unwrap(),
            RevisionFileContent::Truncated
        );
        assert_eq!(
            revision_file_content(truncated, "missing.txt").unwrap(),
            RevisionFileContent::Absent
        );
    }

    #[test]
    fn parses_stargazer_counts_graphql_aliases() {
        let raw = r#"{
            "data": {
                "n0": {"name": "abc123", "stargazerCount": 3},
                "n1": null,
                "n2": {"name": "def456", "stargazerCount": 0}
            }
        }"#;
        let counts = parse_stargazer_counts_graphql(raw).unwrap();
        assert_eq!(counts.get("abc123").copied(), Some(3));
        assert!(!counts.contains_key("def456"));
    }

    #[test]
    fn build_stargazer_graphql_query_aliases_nodes() {
        let q = build_stargazer_graphql_query(&["G_a".into(), "G_b".into()]);
        assert!(q.contains(r#"n0: node(id: "G_a")"#));
        assert!(q.contains(r#"n1: node(id: "G_b")"#));
        assert!(q.contains("stargazerCount"));
    }

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
    fn build_gist_revision_raw_url_includes_owner_when_known() {
        let url = build_gist_revision_raw_url("karpathy", "abc123", "deadbeef", "notes.md");
        assert_eq!(
            url,
            "https://gist.githubusercontent.com/karpathy/abc123/raw/deadbeef/notes.md"
        );
    }

    #[test]
    fn fetch_revision_file_falls_back_when_revision_api_fails() {
        use crate::actions::{CommandOutput, CommandPlan, CommandRunner};
        use std::cell::RefCell;

        struct SeqRunner {
            outputs: RefCell<Vec<CommandOutput>>,
            calls: RefCell<Vec<CommandPlan>>,
            next: RefCell<usize>,
        }

        impl CommandRunner for SeqRunner {
            fn run(&self, plan: &CommandPlan) -> Result<CommandOutput> {
                self.calls.borrow_mut().push(plan.clone());
                let i = *self.next.borrow();
                *self.next.borrow_mut() = i + 1;
                self.outputs
                    .borrow()
                    .get(i)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no output for call {i}"))
            }
        }

        let url = build_gist_revision_raw_url("karpathy", "g1", "sha1", "f.md");
        let runner = SeqRunner {
            outputs: RefCell::new(vec![
                CommandOutput {
                    success: false,
                    stdout: String::new(),
                    stderr: "HTTP 502".into(),
                },
                CommandOutput {
                    success: true,
                    stdout: "revision body".into(),
                    stderr: String::new(),
                },
            ]),
            calls: RefCell::new(Vec::new()),
            next: RefCell::new(0),
        };

        let content = fetch_revision_file_with(&runner, "g1", "sha1", "f.md", "karpathy").unwrap();
        assert_eq!(
            content,
            RevisionFileContent::Present("revision body".into())
        );
        let calls = runner.calls.borrow();
        assert_eq!(calls[0], gist_revision_plan("g1", "sha1"));
        assert_eq!(calls[1], raw_url_fetch_plan(&url));
    }

    #[test]
    fn fetch_gist_file_content_falls_back_to_raw_url() {
        use crate::actions::{CommandOutput, CommandPlan, CommandRunner};
        use std::cell::RefCell;

        struct SeqRunner {
            outputs: RefCell<Vec<CommandOutput>>,
            calls: RefCell<Vec<CommandPlan>>,
            next: RefCell<usize>,
        }

        impl CommandRunner for SeqRunner {
            fn run(&self, plan: &CommandPlan) -> Result<CommandOutput> {
                self.calls.borrow_mut().push(plan.clone());
                let i = *self.next.borrow();
                *self.next.borrow_mut() = i + 1;
                self.outputs
                    .borrow()
                    .get(i)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no output for call {i}"))
            }
        }

        let url = "https://gist.githubusercontent.com/u/id/raw/hash/file.md";
        let runner = SeqRunner {
            outputs: RefCell::new(vec![
                CommandOutput {
                    success: false,
                    stdout: String::new(),
                    stderr: "HTTP 502".into(),
                },
                CommandOutput {
                    success: true,
                    stdout: "big content".into(),
                    stderr: String::new(),
                },
            ]),
            calls: RefCell::new(Vec::new()),
            next: RefCell::new(0),
        };

        let content = fetch_gist_file_content_with(&runner, "id", "file.md", Some(url)).unwrap();
        assert_eq!(content, "big content");
        let calls = runner.calls.borrow();
        assert_eq!(calls[0], gist_view_plan("id", "file.md"));
        assert_eq!(calls[1], raw_url_fetch_plan(url));
    }

    #[test]
    fn parses_comment_counts_defaulting_to_zero() {
        let raw = include_str!("../tests/fixtures/gh/gist-list.json");
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
