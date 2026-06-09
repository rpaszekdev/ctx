use crate::config::CtxConfig;
use crate::differ;
use crate::parser;
use crate::paths;
use crate::resolver;
use crate::rippler;
use crate::state::{CtxState, LogEntry};
use crate::walker;
use std::path::Path;

pub fn full_scan(cfg: &CtxConfig, state: &mut CtxState) -> usize {
    let files = walker::collect_files(&cfg.project_root, cfg);
    let mut total = 0;

    for file in &files {
        if let Ok(content) = std::fs::read_to_string(file) {
            let symbols = parser::extract_symbols(&content, file);
            let imports = parser::extract_imports(&content, file);
            let rel = file.strip_prefix(&cfg.project_root).unwrap_or(file);
            let arch_path = paths::to_arch_path(rel);

            let resolved = resolver::resolve_imports(&imports, file, &cfg.project_root);
            state.graph.set_imports(file.clone(), resolved);

            for sym in &symbols {
                let fqn = format!("{}/{}", arch_path, sym.name);
                state.add_symbol(fqn, sym.name.clone(), sym.kind.clone(), file.clone(), sym.line, 0.5);
                total += 1;
            }

            state.update_file_cache(file.clone(), Some(content));
        }
    }

    state.build_dependency_links();
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
            state.update_file_cache(file.to_path_buf(), Some(content));
        }
        return;
    }

    let rel = file.strip_prefix(&cfg.project_root).unwrap_or(file);
    let arch = paths::to_arch_path(rel);
    let module = paths::to_module(&arch);
    state.record_module_write(&module);
    state.record_file_write(file);

    for change in &changes {
        match change.diff_kind {
            differ::DiffKind::Added => {
                state.add_symbol(
                    change.fqn.clone(),
                    change.name.clone(),
                    change.kind.clone(),
                    file.to_path_buf(),
                    change.line,
                    1.0,
                );
            }
            differ::DiffKind::Modified => {
                state.touch_symbol(&change.fqn, change.line);
            }
            differ::DiffKind::Removed => {
                state.remove_symbol(&change.fqn);
            }
        }

        let rippled = rippler::ripple(state, &file.to_path_buf(), 0.5, cfg.max_ripple_depth);

        state.record_change(LogEntry {
            timestamp: now,
            op: change.diff_kind.to_string(),
            path: change.fqn.clone(),
            detail: change.detail.clone(),
            trail_strength: state.symbols.get(&change.fqn).map_or(0.0, |s| s.total_heat()),
            rippled_to: rippled,
        });
    }

    if let Some(new_src) = &new_content {
        let imports = parser::extract_imports(new_src, file);
        let resolved = resolver::resolve_imports(&imports, file, &cfg.project_root);
        state.graph.set_imports(file.to_path_buf(), resolved);
    }

    state.update_file_cache(file.to_path_buf(), new_content);
    state.build_dependency_links();
}
