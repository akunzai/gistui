//! Unique scratch directories under the system temp dir with guaranteed cleanup.
//!
//! Upload, restore-revision, and gist compaction each need a short-lived work
//! directory. Hand-rolling create + `remove_dir_all` at every site made it easy
//! to miss a failure path (issue #275). This module owns that pattern once:
//!
//! - [`with_temp_scratch_dir`] — sync create → run → always clean up (e.g. compact)
//! - [`ScratchDir`] — same ownership as RAII when the path must outlive the
//!   creating scope (upload/restore write on the UI thread, consume in a bg job)

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// RAII unique scratch directory under the system temp dir.
///
/// Best-effort removed on drop, so early returns and ownership moved into a
/// background job both clean up without a second manual `remove_dir_all`.
#[derive(Debug)]
pub struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    /// Create a unique empty directory named `.gistui_{kind}_{pid}_{stamp}_{seq}`.
    ///
    /// `create_dir` refuses an existing directory rather than adopting it, so two
    /// live `ScratchDir`s can never share a path — otherwise one's Drop deletes the
    /// other's files. The timestamp alone did not guarantee that: two calls with the
    /// same `kind` inside one clock tick produced the same name.
    pub fn create(kind: &str) -> std::io::Result<Self> {
        loop {
            let path = unique_path(kind);
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                // A fresh `seq` every pass, so the retry cannot spin.
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Create a unique scratch directory, run `f`, and always clean up afterward.
///
/// Cleanup runs even when `f` returns `Err` (Drop of the internal [`ScratchDir`]).
/// Create failures surface as `E` via [`From<std::io::Error>`].
///
/// Prefer this for fully-synchronous work. When the scratch path must live past
/// the creating scope (e.g. into a background job), use [`ScratchDir`] instead
/// and move it into that scope — Drop still owns cleanup.
pub fn with_temp_scratch_dir<T, E, F>(kind: &str, f: F) -> Result<T, E>
where
    F: FnOnce(&Path) -> Result<T, E>,
    E: From<std::io::Error>,
{
    let dir = ScratchDir::create(kind)?;
    f(dir.path())
}

/// Per-process counter, so uniqueness does not rest on the clock's resolution.
static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique_path(kind: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let safe: String = kind
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let kind = if safe.is_empty() {
        "scratch"
    } else {
        safe.as_str()
    };
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        ".gistui_{kind}_{}_{stamp}_{seq}",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn with_temp_scratch_dir_creates_dir_and_cleans_up_on_success() {
        let mut seen = PathBuf::new();
        let result: Result<(), io::Error> = with_temp_scratch_dir("test_ok", |dir| {
            assert!(dir.is_dir(), "scratch dir should exist for the body");
            assert!(
                dir.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.contains("test_ok")),
                "kind should appear in the directory name"
            );
            seen = dir.to_path_buf();
            fs::write(dir.join("payload.txt"), b"hi")?;
            Ok(())
        });
        assert!(result.is_ok());
        assert!(
            !seen.exists(),
            "scratch dir must be removed after the body returns Ok"
        );
    }

    #[test]
    fn with_temp_scratch_dir_cleans_up_on_body_error() {
        // Spec: cleanup-on-write-failure (and any early Err) is owned by the helper,
        // so callers cannot forget remove_dir_all (issue #275).
        let mut seen = PathBuf::new();
        let result: Result<(), io::Error> = with_temp_scratch_dir("test_err", |dir| {
            seen = dir.to_path_buf();
            fs::write(dir.join("partial.txt"), b"partial")?;
            Err(io::Error::other("simulated write/body failure"))
        });
        assert!(result.is_err());
        assert!(
            !seen.exists(),
            "scratch dir must be removed even when the body returns Err"
        );
    }

    #[test]
    fn scratch_dir_drop_cleans_up_after_move() {
        // Models the upload/restore pattern: create on the UI thread, move into a
        // background job, drop after the job finishes.
        let path = {
            let scratch = ScratchDir::create("test_move").expect("create");
            let path = scratch.path().to_path_buf();
            assert!(path.is_dir());
            fs::write(path.join("file.txt"), b"x").unwrap();
            // Move ownership out of this block the way a `move ||` closure would.
            let held = scratch;
            assert!(path.is_dir(), "dir must survive while ScratchDir is alive");
            drop(held);
            path
        };
        assert!(!path.exists(), "drop after move must still clean up");
    }

    #[test]
    fn unique_paths_differ_across_calls() {
        let a = ScratchDir::create("uniq").expect("create a");
        let b = ScratchDir::create("uniq").expect("create b");
        assert_ne!(a.path(), b.path());
    }

    #[test]
    fn concurrent_creates_of_one_kind_do_not_share_a_path() {
        // Regression: the name was pid + timestamp only, so two `restore` scratch
        // dirs created inside one clock tick landed on the same path. The second
        // adopted the first's directory, and its Drop deleted the first's payload
        // out from under a live job. Same kind, same instant, on purpose.
        const THREADS: usize = 16;
        let start = std::sync::Arc::new(std::sync::Barrier::new(THREADS));

        let paths: Vec<PathBuf> = (0..THREADS)
            .map(|i| {
                let start = std::sync::Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    let scratch = ScratchDir::create("same").expect("create");
                    let path = scratch.path().join("payload.txt");
                    fs::write(&path, i.to_string().as_bytes()).expect("write");
                    // Every other thread's dir is still live at this point, so a
                    // shared path shows up as a lost or overwritten payload.
                    assert_eq!(fs::read_to_string(&path).expect("read"), i.to_string());
                    scratch.path().to_path_buf()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().expect("thread"))
            .collect();

        let distinct: std::collections::HashSet<&PathBuf> = paths.iter().collect();
        assert_eq!(
            distinct.len(),
            THREADS,
            "every scratch dir needs its own path"
        );
    }
}
