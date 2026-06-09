use crate::parser;
use crate::paths;
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
    let arch_path = paths::to_arch_path(rel);

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
                    Some(os) if os.body_hash != ns.body_hash => changes.push(SymbolChange {
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
