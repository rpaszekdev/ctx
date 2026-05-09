use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CtxConfig {
    pub project_root: PathBuf,
    pub ctx_path: PathBuf,
    pub watch_extensions: Vec<String>,
    pub ignore_dirs: Vec<String>,
    pub decay_factor: f64,
    pub decay_interval_secs: u64,
    pub debounce_ms: u64,
    pub max_ripple_depth: u8,
    pub compaction_threshold: f64,
}

impl CtxConfig {
    pub fn new(project_root: PathBuf) -> Self {
        let ctx_path = project_root.join(".ctx");
        Self {
            project_root,
            ctx_path,
            watch_extensions: vec![
                "ts".into(), "tsx".into(), "js".into(), "jsx".into(),
                "rs".into(), "py".into(), "go".into(),
            ],
            ignore_dirs: vec![
                "node_modules".into(), ".git".into(), "target".into(),
                "dist".into(), ".next".into(), "__pycache__".into(),
                ".ctx".into(),
            ],
            decay_factor: 0.9,
            decay_interval_secs: 600,
            debounce_ms: 300,
            max_ripple_depth: 3,
            compaction_threshold: 0.01,
        }
    }

    pub fn should_watch(&self, path: &std::path::Path) -> bool {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !self.watch_extensions.iter().any(|w| w == ext) {
            return false;
        }
        for component in path.components() {
            let name = component.as_os_str().to_string_lossy();
            if self.ignore_dirs.iter().any(|d| d == name.as_ref()) {
                return false;
            }
        }
        true
    }
}
