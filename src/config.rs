use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CtxConfig {
    #[serde(skip)]
    pub project_root: PathBuf,
    #[serde(skip)]
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
        let config_path = ctx_path.join("config.toml");

        let mut cfg = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path).unwrap_or_default();
            toml::from_str(&content).unwrap_or_else(|e| {
                eprintln!("Warning: invalid config.toml ({}), using defaults", e);
                Self::defaults()
            })
        } else {
            Self::defaults()
        };

        cfg.project_root = project_root;
        cfg.ctx_path = ctx_path;
        cfg
    }

    fn defaults() -> Self {
        Self {
            project_root: PathBuf::new(),
            ctx_path: PathBuf::new(),
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

    pub fn write_default_config(ctx_path: &PathBuf) {
        let cfg = Self::defaults();
        let content = toml::to_string_pretty(&cfg).expect("failed to serialize config");
        std::fs::write(ctx_path.join("config.toml"), content).expect("failed to write config.toml");
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
