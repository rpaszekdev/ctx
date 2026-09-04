use super::{ParsedImport, ParsedSymbol};
use regex::Regex;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Extracts `import a.b` and `from a.b import c` (including relative `from .x`).
///
/// `source` keeps the dotted form verbatim, leading dots and all — the resolver
/// needs the dot count to know how many directories to climb. Stdlib and
/// third-party modules are extracted too; they simply resolve to no file and
/// drop out, which is cheaper than maintaining a list of what to ignore.
pub fn extract_imports(src: &str) -> Vec<ParsedImport> {
    let import_re = Regex::new(r"^import\s+([\w\.]+)").unwrap();
    let from_re = Regex::new(r"^from\s+([\w\.]+)\s+import\s+(.+)$").unwrap();
    let mut imports = Vec::new();

    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }

        if let Some(caps) = from_re.captures(trimmed) {
            imports.push(ParsedImport {
                source: caps[1].to_string(),
                names: parse_imported_names(&caps[2]),
            });
        } else if let Some(caps) = import_re.captures(trimmed) {
            imports.push(ParsedImport {
                source: caps[1].to_string(),
                names: Vec::new(),
            });
        }
    }

    imports
}

/// `a, b as c, (d,` → `["a", "b", "d"]`. A name may be a submodule rather than a
/// symbol, so the resolver tries it as a path before falling back to the parent.
fn parse_imported_names(clause: &str) -> Vec<String> {
    clause
        .trim_start_matches('(')
        .split(',')
        .filter_map(|part| {
            let name = part.trim().split_whitespace().next()?;
            let name = name.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if name.is_empty() || name == "*" {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect()
}

pub fn extract_symbols(src: &str) -> Vec<ParsedSymbol> {
    let mut raw_symbols = Vec::new();
    let class_re = Regex::new(r"^class\s+(\w+)").unwrap();
    let func_re = Regex::new(r"^(?:async\s+)?def\s+(\w+)").unwrap();

    let lines: Vec<&str> = src.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }

        let sym = if let Some(caps) = class_re.captures(trimmed) {
            Some((caps[1].to_string(), "type".to_string(), i + 1))
        } else if let Some(caps) = func_re.captures(trimmed) {
            let name = caps[1].to_string();
            if name.starts_with("__") && name.ends_with("__") {
                None
            } else {
                Some((name, "function".to_string(), i + 1))
            }
        } else {
            None
        };

        if let Some(s) = sym {
            raw_symbols.push(s);
        }
    }

    let total_lines = lines.len();
    raw_symbols.iter().enumerate().map(|(i, (name, kind, line))| {
        let start = line - 1;
        let end = if i + 1 < raw_symbols.len() {
            (raw_symbols[i + 1].2 - 1).min(total_lines)
        } else {
            total_lines
        };
        let body = lines[start..end].join("\n");
        let mut hasher = DefaultHasher::new();
        body.hash(&mut hasher);

        ParsedSymbol {
            name: name.clone(),
            kind: kind.clone(),
            line: *line,
            body_hash: hasher.finish(),
        }
    }).collect()
}
