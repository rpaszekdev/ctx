//! Worker attribution.
//!
//! Filesystem events tell the daemon *what* changed but never *who* changed it,
//! and only the worker knows its own name — so attribution has to be pushed in
//! by `ctx touch`.
//!
//! Trails are an append-only sidecar (`.ctx/trails.jsonl`), never part of
//! `.state`. The daemon owns `.state` exclusively; a worker that also wrote it
//! would race the daemon's read-modify-write cycle and clobber heat. Keeping the
//! two streams separate means attribution is a pure read-time join: a change to
//! `foo/bar` at time T belongs to the worker whose trail names `foo/bar` closest
//! to T.

use crate::config::CtxConfig;
use crate::paths;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A change is attributed to a trail recorded within this many seconds of it.
/// Workers call `ctx touch` after editing, so the trail normally lands a few
/// seconds *after* the daemon logged the write — the window spans both sides.
const JOIN_WINDOW_SECS: i64 = 120;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trail {
    pub worker: String,
    pub timestamp: i64,
    /// Architectural paths (extension-stripped), matching `LogEntry::path` prefixes.
    pub arch_paths: Vec<String>,
    pub files: Vec<String>,
}

fn trails_path(cfg: &CtxConfig) -> std::path::PathBuf {
    cfg.ctx_path.join("trails.jsonl")
}

/// Append one trail. Never rewrites earlier lines, so concurrent workers can
/// record at the same time without clobbering each other.
pub fn append(cfg: &CtxConfig, trail: &Trail) -> std::io::Result<()> {
    use std::io::Write;

    let line = serde_json::to_string(trail).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(trails_path(cfg))?;
    writeln!(file, "{}", line)
}

/// Read every trail. Unparseable lines are skipped rather than failing the read —
/// a torn final line must not hide the history behind it.
pub fn load(cfg: &CtxConfig) -> Vec<Trail> {
    let Ok(content) = std::fs::read_to_string(trails_path(cfg)) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// The worker responsible for a change to `log_path` at `timestamp`, if any.
///
/// `log_path` is a `LogEntry::path` — an fqn like `app/module/symbolName`, whose
/// prefix is the file's architectural path. Matching on that prefix rather than
/// resolving the symbol means removed symbols still attribute correctly.
pub fn attribute(trails: &[Trail], log_path: &str, timestamp: i64) -> Option<String> {
    let arch = log_path.rsplit_once('/').map(|(head, _)| head)?;

    trails
        .iter()
        .filter(|t| (t.timestamp - timestamp).abs() <= JOIN_WINDOW_SECS)
        .filter(|t| t.arch_paths.iter().any(|p| p == arch))
        .min_by_key(|t| (t.timestamp - timestamp).abs())
        .map(|t| t.worker.clone())
}

/// Architectural path for a file, for matching against `LogEntry::path` prefixes.
pub fn arch_path_of(file: &Path, project_root: &Path) -> String {
    let rel = file.strip_prefix(project_root).unwrap_or(file);
    paths::to_arch_path(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trail(worker: &str, ts: i64, arch: &[&str]) -> Trail {
        Trail {
            worker: worker.to_string(),
            timestamp: ts,
            arch_paths: arch.iter().map(|s| s.to_string()).collect(),
            files: Vec::new(),
        }
    }

    #[test]
    fn attributes_a_change_to_the_worker_that_touched_that_file() {
        let trails = vec![trail("impl-auth", 1000, &["app/auth"])];
        assert_eq!(
            attribute(&trails, "app/auth/validate", 995).as_deref(),
            Some("impl-auth")
        );
    }

    #[test]
    fn picks_the_nearest_worker_when_two_touched_the_same_file() {
        let trails = vec![
            trail("early", 900, &["app/auth"]),
            trail("late", 1010, &["app/auth"]),
        ];
        assert_eq!(
            attribute(&trails, "app/auth/validate", 1000).as_deref(),
            Some("late")
        );
    }

    #[test]
    fn does_not_attribute_across_the_join_window() {
        let trails = vec![trail("stale", 0, &["app/auth"])];
        assert_eq!(attribute(&trails, "app/auth/validate", 10_000), None);
    }

    #[test]
    fn does_not_attribute_a_file_the_worker_never_touched() {
        let trails = vec![trail("impl-auth", 1000, &["app/auth"])];
        assert_eq!(attribute(&trails, "app/billing/charge", 1000), None);
    }

    #[test]
    fn a_torn_line_does_not_hide_the_trails_before_it() {
        let dir = std::env::temp_dir().join("ctx-trails-torn-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".ctx")).unwrap();
        let cfg = CtxConfig::new(dir);

        append(&cfg, &trail("good", 1000, &["app/auth"])).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(cfg.ctx_path.join("trails.jsonl"))
            .map(|mut f| {
                use std::io::Write;
                writeln!(f, "{{\"worker\": \"trunc")
            })
            .unwrap()
            .unwrap();

        let trails = load(&cfg);
        assert_eq!(trails.len(), 1);
        assert_eq!(trails[0].worker, "good");
    }

    #[test]
    fn appending_twice_keeps_both() {
        let dir = std::env::temp_dir().join("ctx-trails-append-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".ctx")).unwrap();
        let cfg = CtxConfig::new(dir);

        append(&cfg, &trail("w1", 1000, &["app/a"])).unwrap();
        append(&cfg, &trail("w1", 2000, &["app/b"])).unwrap();

        let trails = load(&cfg);
        assert_eq!(trails.len(), 2, "second touch must not overwrite the first");
    }
}
