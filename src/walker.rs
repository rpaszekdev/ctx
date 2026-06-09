use crate::config::CtxConfig;
use std::path::{Path, PathBuf};

pub fn collect_files(dir: &Path, cfg: &CtxConfig) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk(dir, cfg, &mut files);
    files
}

fn walk(dir: &Path, cfg: &CtxConfig, files: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if cfg.ignore_dirs.iter().any(|d| d == &name) || name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            walk(&path, cfg, files);
        } else if cfg.should_watch(&path) {
            files.push(path);
        }
    }
}
