//! Stargazer counts via a batched GraphQL query (issue #301).

use super::escape_graphql_string;
use crate::actions::{run_command, CommandPlan, CommandRunner};
use anyhow::{Context, Result};
use std::collections::HashMap;

const STARGAZER_GRAPHQL_CHUNK: usize = 40;

/// Build a batched GraphQL query (`n0`…`n{k}` aliases) for stargazer counts.
pub fn build_stargazer_graphql_query(node_ids: &[String]) -> String {
    let mut query = String::from("query { ");
    for (i, id) in node_ids.iter().enumerate() {
        let id = escape_graphql_string(id);
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

pub fn collect_gist_star_counts(
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn build_stargazer_graphql_query_escapes_quotes_and_backslashes() {
        // A node_id carrying a double-quote or backslash must stay inside its string
        // literal rather than break out of the query (defensive against malformed ids).
        let q = build_stargazer_graphql_query(&["a\"b".into(), "c\\d".into()]);
        assert!(q.contains(r#"n0: node(id: "a\"b")"#));
        assert!(q.contains(r#"n1: node(id: "c\\d")"#));
        // The injected quote does not prematurely close the literal: a break-out attempt
        // like `") {} #` stays escaped, so no bare `) {` from the payload appears.
        let inj = build_stargazer_graphql_query(&["x\") { __typename } #".into()]);
        assert!(inj.contains(r#"node(id: "x\") { __typename } #")"#));
    }
}
