use crate::config::CtxConfig;
use crate::differ;
use crate::parser;
use crate::rate;
use crate::rippler;
use crate::state::{CtxState, LogEntry, TrackedSymbol};
use std::path::{Path, PathBuf};

pub fn full_scan(cfg: &CtxConfig, state: &mut CtxState) -> usize {
    let files = collect_files(&cfg.project_root, cfg);
    let mut total = 0;

    for file in &files {
        if let Ok(content) = std::fs::read_to_string(file) {
            let symbols = parser::extract_symbols(&content, file);
            let imports = parser::extract_imports(&content, file);
            let rel = file.strip_prefix(&cfg.project_root).unwrap_or(file);
            let arch_path = to_arch_path(rel);

            let resolved_imports = resolve_imports(&imports, file, &cfg.project_root);
            state.graph.set_imports(file.clone(), resolved_imports);

            for sym in &symbols {
                let fqn = format!("{}/{}", arch_path, sym.name);
                let now = chrono::Utc::now().timestamp();
                state.symbols.insert(fqn.clone(), TrackedSymbol {
                    name: sym.name.clone(),
                    kind: sym.kind.clone(),
                    fqn,
                    file: file.clone(),
                    line: sym.line,
                    trail_strength: 0.5,
                    last_touched: now,
                    touch_count: 0,
                    used_by: Vec::new(),
                    uses: Vec::new(),
                });
                total += 1;
            }

            state.file_cache.insert(file.clone(), content);
        }
    }

    build_dependency_links(state);
    total
}

pub fn process_file_change(cfg: &CtxConfig, state: &mut CtxState, file: &Path) {
    let old_content = state.file_cache.get(&file.to_path_buf()).cloned();
    let new_content = std::fs::read_to_string(file).ok();
    let now = chrono::Utc::now().timestamp();

    let changes = differ::diff_file(
        old_content.as_deref(),
        new_content.as_deref(),
        file,
        &cfg.project_root,
    );

    if changes.is_empty() {
        if let Some(content) = new_content {
            state.file_cache.insert(file.to_path_buf(), content);
        }
        return;
    }

    let module = {
        let rel = file.strip_prefix(&cfg.project_root).unwrap_or(file);
        let arch = to_arch_path(rel);
        let parts: Vec<&str> = arch.split('/').collect();
        if parts.len() >= 2 { parts[..2].join("/") } else { parts.join("/") }
    };
    rate::record_write(&mut state.rates, &module);

    for change in &changes {
        match change.diff_kind {
            differ::DiffKind::Added => {
                state.symbols.insert(change.fqn.clone(), TrackedSymbol {
                    name: change.name.clone(),
                    kind: change.kind.clone(),
                    fqn: change.fqn.clone(),
                    file: file.to_path_buf(),
                    line: change.line,
                    trail_strength: 1.0,
                    last_touched: now,
                    touch_count: 1,
                    used_by: Vec::new(),
                    uses: Vec::new(),
                });
            }
            differ::DiffKind::Modified => {
                if let Some(sym) = state.symbols.get_mut(&change.fqn) {
                    sym.trail_strength = (sym.trail_strength + 1.0).min(5.0);
                    sym.last_touched = now;
                    sym.touch_count += 1;
                    sym.line = change.line;
                }
            }
            differ::DiffKind::Removed => {
                state.symbols.remove(&change.fqn);
            }
        }

        let rippled = rippler::ripple(state, &file.to_path_buf(), 0.5, cfg.max_ripple_depth);

        state.log.push(LogEntry {
            timestamp: now,
            op: change.diff_kind.to_string(),
            path: change.fqn.clone(),
            detail: change.detail.clone(),
            trail_strength: state.symbols.get(&change.fqn).map_or(0.0, |s| s.trail_strength),
            rippled_to: rippled,
        });
    }

    if let Some(new_src) = &new_content {
        let imports = parser::extract_imports(new_src, file);
        let resolved = resolve_imports(&imports, file, &cfg.project_root);
        state.graph.set_imports(file.to_path_buf(), resolved);
    }

    if let Some(content) = new_content {
        state.file_cache.insert(file.to_path_buf(), content);
    } else {
        state.file_cache.remove(&file.to_path_buf());
    }

    build_dependency_links(state);
}

pub fn decay_all(state: &mut CtxState, factor: f64, threshold: f64) {
    let mut to_remove = Vec::new();
    for (fqn, sym) in state.symbols.iter_mut() {
        sym.trail_strength *= factor;
        if sym.trail_strength < threshold {
            to_remove.push(fqn.clone());
        }
    }
    for fqn in to_remove {
        state.symbols.remove(&fqn);
    }
}

fn collect_files(dir: &Path, cfg: &CtxConfig) -> Vec<PathBuf> {
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

fn to_arch_path(rel: &Path) -> String {
    rel.to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("src/")
        .trim_end_matches(".ts").trim_end_matches(".tsx")
        .trim_end_matches(".js").trim_end_matches(".jsx")
        .trim_end_matches(".rs").trim_end_matches(".py")
        .trim_end_matches(".go")
        .to_string()
}

fn resolve_imports(imports: &[parser::ParsedImport], from_file: &Path, project_root: &Path) -> Vec<PathBuf> {
    let dir = from_file.parent().unwrap_or(project_root);
    let extensions = ["ts", "tsx", "js", "jsx"];
    let mut resolved = Vec::new();

    for imp in imports {
        let base = dir.join(&imp.source);
        for ext in &extensions {
            let candidate = base.with_extension(ext);
            if candidate.exists() {
                resolved.push(candidate);
                break;
            }
            let index = base.join(format!("index.{}", ext));
            if index.exists() {
                resolved.push(index);
                break;
            }
        }
    }

    resolved
}

fn build_dependency_links(state: &mut CtxState) {
    let graph_snapshot: Vec<(PathBuf, Vec<PathBuf>)> = state.graph.edges.iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let file_to_fqns: std::collections::HashMap<PathBuf, Vec<String>> = {
        let mut map: std::collections::HashMap<PathBuf, Vec<String>> = std::collections::HashMap::new();
        for sym in state.symbols.values() {
            map.entry(sym.file.clone()).or_default().push(sym.fqn.clone());
        }
        map
    };

    for sym in state.symbols.values_mut() {
        sym.uses.clear();
        sym.used_by.clear();
    }

    for (file, imports) in &graph_snapshot {
        let source_fqns = file_to_fqns.get(file).cloned().unwrap_or_default();
        for imported_file in imports {
            let target_fqns = file_to_fqns.get(imported_file).cloned().unwrap_or_default();
            for src_fqn in &source_fqns {
                for tgt_fqn in &target_fqns {
                    if let Some(sym) = state.symbols.get_mut(src_fqn) {
                        if !sym.uses.contains(tgt_fqn) {
                            sym.uses.push(tgt_fqn.clone());
                        }
                    }
                    if let Some(sym) = state.symbols.get_mut(tgt_fqn) {
                        if !sym.used_by.contains(src_fqn) {
                            sym.used_by.push(src_fqn.clone());
                        }
                    }
                }
            }
        }
    }
}
