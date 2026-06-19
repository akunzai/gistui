//! Integration tests for the `gh` command boundary.
//!
//! These exercise command planning, success/failure handling, and output
//! parsing end to end through a fake [`CommandRunner`] implemented here, in a
//! separate crate from the library — proving the boundary is genuinely
//! injectable. No test touches the network or requires `gh` to be installed.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;

use gistui::actions::{run_command, upload_command, CommandOutput, CommandPlan, CommandRunner};
use gistui::domain::GistFile;
use gistui::gh::{
    auth_status_plan, check_gh_ready_with, fetch_gist_comments_json_with,
    fetch_gist_file_content_with, fetch_gist_list_json_with, gh_version_plan, gist_comments_plan,
    gist_list_plan, gist_view_plan, parse_gist_comments_json, parse_gist_list_json,
};

/// A scripted runner: returns queued outputs in order and records every plan it
/// received so tests can assert which `gh` command would have run.
struct FakeRunner {
    queue: RefCell<VecDeque<CommandOutput>>,
    calls: RefCell<Vec<CommandPlan>>,
}

impl FakeRunner {
    fn new(outputs: Vec<CommandOutput>) -> Self {
        FakeRunner {
            queue: RefCell::new(outputs.into()),
            calls: RefCell::new(Vec::new()),
        }
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            success: true,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    fn fail(stderr: &str) -> CommandOutput {
        CommandOutput {
            success: false,
            stdout: String::new(),
            stderr: stderr.to_string(),
        }
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, plan: &CommandPlan) -> anyhow::Result<CommandOutput> {
        self.calls.borrow_mut().push(plan.clone());
        Ok(self
            .queue
            .borrow_mut()
            .pop_front()
            .expect("FakeRunner ran out of scripted outputs"))
    }
}

const GIST_LIST_JSON: &str = include_str!("fixtures/gh/gist-list.json");
const GIST_COMMENTS_JSON: &str = include_str!("fixtures/gh/gist-comments.json");

#[test]
fn fetch_and_parse_gist_list_via_fake_runner() {
    let runner = FakeRunner::new(vec![FakeRunner::ok(GIST_LIST_JSON)]);

    let raw = fetch_gist_list_json_with(&runner).unwrap();
    let files = parse_gist_list_json(&raw).unwrap();

    assert_eq!(files.len(), 3);
    assert_eq!(runner.calls.borrow()[0], gist_list_plan());
}

#[test]
fn fetch_gist_list_surfaces_stderr_on_failure() {
    let runner = FakeRunner::new(vec![FakeRunner::fail("HTTP 401: Bad credentials")]);

    let err = fetch_gist_list_json_with(&runner).unwrap_err();
    assert!(err.to_string().contains("Bad credentials"));
}

#[test]
fn fetch_gist_file_content_returns_stdout_and_plans_view() {
    let runner = FakeRunner::new(vec![FakeRunner::ok("hello = true\n")]);

    let content = fetch_gist_file_content_with(&runner, "abc123", "config.toml", None).unwrap();

    assert_eq!(content, "hello = true\n");
    assert_eq!(
        runner.calls.borrow()[0],
        gist_view_plan("abc123", "config.toml")
    );
}

#[test]
fn fetch_gist_file_content_surfaces_stderr_on_failure() {
    let runner = FakeRunner::new(vec![FakeRunner::fail("could not find gist")]);

    let err = fetch_gist_file_content_with(&runner, "missing", "x", None).unwrap_err();
    assert!(err.to_string().contains("could not find gist"));
}

#[test]
fn check_gh_ready_passes_when_version_and_auth_succeed() {
    let runner = FakeRunner::new(vec![FakeRunner::ok(""), FakeRunner::ok("")]);

    assert!(check_gh_ready_with(&runner).is_ok());

    let calls = runner.calls.borrow();
    assert_eq!(calls[0], gh_version_plan());
    assert_eq!(calls[1], auth_status_plan());
}

#[test]
fn check_gh_ready_fails_when_gh_missing() {
    let runner = FakeRunner::new(vec![FakeRunner::fail("command not found")]);

    let err = check_gh_ready_with(&runner).unwrap_err();
    assert!(err.to_string().contains("did not run successfully"));
    // Auth is never probed once the version check fails.
    assert_eq!(runner.calls.borrow().len(), 1);
}

#[test]
fn check_gh_ready_fails_when_unauthenticated() {
    let runner = FakeRunner::new(vec![FakeRunner::ok(""), FakeRunner::fail("not logged in")]);

    let err = check_gh_ready_with(&runner).unwrap_err();
    assert!(err.to_string().contains("gh auth login"));
}

#[test]
fn run_command_executes_planned_write_action() {
    let target = GistFile {
        gist_id: "abc123".into(),
        description: "config".into(),
        filename: "settings.json".into(),
        public: false,
        updated_at: "2026-06-08T00:00:00Z".into(),
        created_at: "2026-06-08T00:00:00Z".into(),
        owner_login: String::new(),
        fork_of_id: None,
        raw_url: None,
    };
    let plan = upload_command(PathBuf::from("/tmp/settings.json").as_path(), &target);
    let runner = FakeRunner::new(vec![FakeRunner::ok("https://gist.github.com/abc123\n")]);

    let stdout = run_command(&runner, &plan).unwrap();

    assert!(stdout.contains("gist.github.com"));
    assert_eq!(runner.calls.borrow()[0], plan);
}

#[test]
fn run_command_surfaces_stderr_on_failure() {
    let plan = CommandPlan {
        program: "gh".into(),
        args: vec![
            "gist".into(),
            "delete".into(),
            "--yes".into(),
            "nope".into(),
        ],
    };
    let runner = FakeRunner::new(vec![FakeRunner::fail("gist not found")]);

    let err = run_command(&runner, &plan).unwrap_err();
    assert!(err.to_string().contains("gist not found"));
}

#[test]
fn fetch_and_parse_gist_comments_via_fake_runner() {
    let runner = FakeRunner::new(vec![FakeRunner::ok(GIST_COMMENTS_JSON)]);

    let raw = fetch_gist_comments_json_with(&runner, "abc123").unwrap();
    let comments = parse_gist_comments_json(&raw).unwrap();

    assert_eq!(comments.len(), 3);
    assert_eq!(comments[0].author, "alice");
    assert_eq!(comments[1].author, "bob");
    assert_eq!(comments[1].body, "Multi-line\nbody here.");
    assert_eq!(comments[2].author, "(unknown)");
    assert_eq!(comments[2].body, "ghost comment");
    assert_eq!(comments[2].created_at, "2026-06-11T01:00:00Z");
    assert_eq!(runner.calls.borrow()[0], gist_comments_plan("abc123"));
}

#[test]
fn fetch_gist_comments_surfaces_stderr_on_failure() {
    let runner = FakeRunner::new(vec![FakeRunner::fail("HTTP 404: Not Found")]);

    let err = fetch_gist_comments_json_with(&runner, "missing").unwrap_err();
    assert!(err.to_string().contains("Not Found"));
}

#[test]
fn parse_gist_comments_handles_empty_array() {
    let comments = parse_gist_comments_json("[]").unwrap();
    assert!(comments.is_empty());
}
