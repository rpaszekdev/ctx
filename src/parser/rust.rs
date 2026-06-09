use super::ParsedSymbol;
use regex::Regex;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub fn extract_symbols(src: &str) -> Vec<ParsedSymbol> {
    let mut raw_symbols = Vec::new();
    let fn_re = Regex::new(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)").unwrap();
    let struct_re = Regex::new(r"^\s*(?:pub\s+)?struct\s+(\w+)").unwrap();
    let enum_re = Regex::new(r"^\s*(?:pub\s+)?enum\s+(\w+)").unwrap();
    let trait_re = Regex::new(r"^\s*(?:pub\s+)?trait\s+(\w+)").unwrap();
    let impl_re = Regex::new(r"^\s*impl(?:<[^>]*>)?\s+(\w+)").unwrap();
    let type_re = Regex::new(r"^\s*(?:pub\s+)?type\s+(\w+)").unwrap();
    let const_re = Regex::new(r"^\s*(?:pub\s+)?const\s+(\w+)").unwrap();
    let mod_re = Regex::new(r"^\s*(?:pub\s+)?mod\s+(\w+)").unwrap();

    let lines: Vec<&str> = src.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with("*") {
            continue;
        }

        let sym = if let Some(caps) = fn_re.captures(trimmed) {
            Some((caps[1].to_string(), "function".to_string(), i + 1))
        } else if let Some(caps) = struct_re.captures(trimmed) {
            Some((caps[1].to_string(), "type".to_string(), i + 1))
        } else if let Some(caps) = enum_re.captures(trimmed) {
            Some((caps[1].to_string(), "type".to_string(), i + 1))
        } else if let Some(caps) = trait_re.captures(trimmed) {
            Some((caps[1].to_string(), "type".to_string(), i + 1))
        } else if let Some(caps) = impl_re.captures(trimmed) {
            if !trimmed.contains(" for ") {
                Some((caps[1].to_string(), "impl".to_string(), i + 1))
            } else {
                let parts: Vec<&str> = trimmed.split(" for ").collect();
                if let Some(target) = parts.get(1) {
                    let name = target.split_whitespace().next().unwrap_or("").trim_matches(|c: char| !c.is_alphanumeric());
                    if !name.is_empty() {
                        Some((format!("{}_{}", caps[1].to_string(), name), "impl".to_string(), i + 1))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        } else if let Some(caps) = type_re.captures(trimmed) {
            Some((caps[1].to_string(), "type".to_string(), i + 1))
        } else if let Some(caps) = const_re.captures(trimmed) {
            Some((caps[1].to_string(), "const".to_string(), i + 1))
        } else if let Some(caps) = mod_re.captures(trimmed) {
            if !trimmed.ends_with(';') {
                Some((caps[1].to_string(), "module".to_string(), i + 1))
            } else {
                None
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
