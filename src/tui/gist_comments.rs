//! The GistDetail comments workflow: the first (newest) page, then older pages on demand.
//! GistDetail owns the intent (`screens::detail::stage_*`) and applies the result
//! (`screens::detail::on_comments_*`); this module stages the job and does the `gh` reads
//! through the injected runner, as `gist_revision` does for its screens.

use super::bg::{ActionJobKind, ActionJobSpec, Jobs};
use super::screens::detail;
use super::AppState;

/// Load `gist_id`'s newest page of comments, unless they are loaded or loading already.
pub(super) fn load_initial(jobs: &mut Jobs, state: &mut AppState, gist_id: String) {
    let Some(gist_id) = detail::stage_fetch_comments(state, gist_id) else {
        return;
    };
    let runner = jobs.command_runner();
    jobs.spawn_action(
        state,
        ActionJobSpec::new(
            ActionJobKind::FetchComments {
                gist_id: gist_id.clone(),
                page: None,
            },
            "Loading comments…",
        ),
        move || {
            let result = crate::gh::fetch_initial_comments(runner.as_ref(), &gist_id)
                .map_err(|e| e.to_string());
            (result, gist_id)
        },
        move |(result, gist_id), state| detail::on_comments_initial_loaded(state, gist_id, result),
    );
}

/// Load comment page `page` of `gist_id`, when an older page remains.
pub(super) fn load_older(jobs: &mut Jobs, state: &mut AppState, gist_id: String, page: u32) {
    let Some(gist_id) = detail::stage_load_older_comments(state, gist_id, page) else {
        return;
    };
    let runner = jobs.command_runner();
    jobs.spawn_action(
        state,
        ActionJobSpec::new(
            ActionJobKind::FetchComments {
                gist_id: gist_id.clone(),
                page: Some(page),
            },
            "Loading older comments…",
        ),
        move || {
            let result = crate::gh::fetch_older_comments(runner.as_ref(), &gist_id, page)
                .map_err(|e| e.to_string());
            (result, gist_id)
        },
        move |(result, gist_id), state| detail::on_comments_older_loaded(state, gist_id, result),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_support::SeqRunner;
    use crate::actions::CommandOutput;
    use crate::tui::{initial_state, DetailState, Screen};
    use std::sync::Arc;

    const COMMENTS: &str = include_str!("../../tests/fixtures/gh/gist-comments.json");

    fn on_detail() -> AppState {
        let mut state = initial_state();
        state.enter(Screen::GistDetail(Box::new(DetailState {
            gist_id: Some("g1".into()),
            ..DetailState::default()
        })));
        state
    }

    fn comment_count(state: &AppState) -> Option<usize> {
        state
            .detail()
            .and_then(|d| d.comments.as_ref())
            .map(Vec::len)
    }

    /// The first load probes the total, then fetches the newest page.
    #[test]
    fn the_first_load_probes_then_fetches_the_newest_page() {
        let mut state = on_detail();
        let runner = Arc::new(SeqRunner::new(vec![
            CommandOutput::ok(format!("HTTP/2.0 200 OK\n\n{COMMENTS}")),
            CommandOutput::ok(COMMENTS),
        ]));
        let mut jobs = Jobs::inline(&state.gist_catalog.clone(), runner.clone());

        load_initial(&mut jobs, &mut state, "g1".into());
        jobs.on_action_outcome(&mut state);

        assert_eq!(
            runner.calls(),
            vec![
                crate::gh::gist_comments_probe_plan("g1"),
                crate::gh::gist_comments_page_plan("g1", 1, crate::gh::COMMENTS_PAGE_SIZE),
            ]
        );
        assert_eq!(comment_count(&state), Some(3));
    }

    /// An older page is fetched and prepended to what is already shown.
    #[test]
    fn an_older_page_is_fetched_and_added() {
        let mut state = on_detail();
        state.apply_initial_comments(
            "g1",
            Ok(crate::gh::InitialComments {
                comments: crate::gh::parse_gist_comments_json(COMMENTS).unwrap(),
                total: 33,
                oldest_page: 2,
            }),
        );
        let runner = Arc::new(SeqRunner::new(vec![CommandOutput::ok(COMMENTS)]));
        let mut jobs = Jobs::inline(&state.gist_catalog.clone(), runner.clone());

        load_older(&mut jobs, &mut state, "g1".into(), 1);
        jobs.on_action_outcome(&mut state);

        assert_eq!(
            runner.calls(),
            vec![crate::gh::gist_comments_page_plan(
                "g1",
                1,
                crate::gh::COMMENTS_PAGE_SIZE
            )]
        );
        assert_eq!(comment_count(&state), Some(6));
        assert!(!state.can_load_older_comments(), "page 1 was the oldest");
    }

    /// A page that is no longer available fetches nothing.
    #[test]
    fn no_older_page_means_no_fetch() {
        let mut state = on_detail();
        let runner = Arc::new(SeqRunner::new(vec![]));
        let mut jobs = Jobs::inline(&state.gist_catalog.clone(), runner.clone());

        load_older(&mut jobs, &mut state, "g1".into(), 1);

        assert!(runner.calls().is_empty());
    }
}
