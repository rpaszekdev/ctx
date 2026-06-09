use super::ParsedSymbol;
use regex::Regex;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

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
