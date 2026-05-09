use crate::parser;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub enum DiffKind {
    Added,
    Removed,
    Modified,
}

impl std::fmt::Display for DiffKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiffKind::Added => write!(f, "W"),
            DiffKind::Removed => write!(f, "D"),
            DiffKind::Modified => write!(f, "W"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SymbolChange {
    pub name: String,
    pub kind: String,
    pub fqn: String,
    pub line: usize,
    pub diff_kind: DiffKind,
    pub detail: String,
}

pub fn diff_file(old: Option<&str>, new: Option<&str>, file_path: &Path, project_root: &Path) -> Vec<SymbolChange> {
    let rel = file_path.strip_prefix(project_root).unwrap_or(file_path);
    let arch_path = rel.to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("src/")
        .trim_end_matches(".ts").trim_end_matches(".tsx")
        .trim_end_matches(".js").trim_end_matches(".jsx")
        .trim_end_matches(".rs").trim_end_matches(".py")
        .trim_end_matches(".go")
        .to_string();

    match (old, new) {
        (None, Some(new_src)) => {
            parser::extract_symbols(new_src, file_path).into_iter().map(|s| {
                SymbolChange {
                    fqn: format!("{}/{}", arch_path, s.name),
                    name: s.name, kind: s.kind, line: s.line,
                    diff_kind: DiffKind::Added, detail: "created".into(),
                }
            }).collect()
        }
        (Some(old_src), None) => {
            parser::extract_symbols(old_src, file_path).into_iter().map(|s| {
                SymbolChange {
                    fqn: format!("{}/{}", arch_path, s.name),
                    name: s.name, kind: s.kind, line: s.line,
                    diff_kind: DiffKind::Removed, detail: "removed".into(),
                }
            }).collect()
        }
        (Some(old_src), Some(new_src)) => {
            let old_syms: HashMap<String, _> = parser::extract_symbols(old_src, file_path)
                .into_iter().map(|s| (s.name.clone(), s)).collect();
            let new_syms: HashMap<String, _> = parser::extract_symbols(new_src, file_path)
                .into_iter().map(|s| (s.name.clone(), s)).collect();
            let mut changes = Vec::new();

            for (name, ns) in &new_syms {
                let fqn = format!("{}/{}", arch_path, name);
                match old_syms.get(name) {
                    None => changes.push(SymbolChange {
                        fqn, name: name.clone(), kind: ns.kind.clone(), line: ns.line,
                        diff_kind: DiffKind::Added, detail: format!("added {}", ns.kind),
                    }),
                    Some(os) if os.line != ns.line => changes.push(SymbolChange {
                        fqn, name: name.clone(), kind: ns.kind.clone(), line: ns.line,
                        diff_kind: DiffKind::Modified, detail: "modified".into(),
                    }),
                    _ => {}
                }
            }

            for (name, os) in &old_syms {
                if !new_syms.contains_key(name) {
                    changes.push(SymbolChange {
                        fqn: format!("{}/{}", arch_path, name),
                        name: name.clone(), kind: os.kind.clone(), line: os.line,
                        diff_kind: DiffKind::Removed, detail: "removed".into(),
                    });
                }
            }

            changes
        }
        _ => vec![],
    }
}
