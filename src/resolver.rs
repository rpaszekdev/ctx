use crate::parser;
use std::path::{Path, PathBuf};

pub fn resolve_imports(imports: &[parser::ParsedImport], from_file: &Path, project_root: &Path) -> Vec<PathBuf> {
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
