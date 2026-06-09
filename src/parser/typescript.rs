use super::{ParsedImport, ParsedSymbol};
use regex::Regex;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub fn extract_symbols(src: &str) -> Vec<ParsedSymbol> {
    let mut raw_symbols = Vec::new();
    let type_re = Regex::new(r"(?:export\s+)?(?:type|interface|enum|class)\s+(\w+)").unwrap();
    let func_re = Regex::new(r"(?:export\s+)?(?:async\s+)?function\s+(\w+)").unwrap();
    let const_fn_re = Regex::new(r"(?:export\s+)?const\s+(\w+)\s*=\s*(?:async\s*)?\(").unwrap();
    let arrow_re = Regex::new(r"(?:export\s+)?const\s+(\w+)\s*=\s*(?:async\s*)?\([^)]*\)\s*(?::\s*\w+\s*)?=>").unwrap();

    let lines: Vec<&str> = src.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with("*") {
            continue;
        }

        let sym = if let Some(caps) = type_re.captures(trimmed) {
            let name = caps[1].to_string();
            let kind = if trimmed.contains("interface") {
                "type"
            } else if trimmed.contains("enum") {
                "type"
            } else if trimmed.contains("class") {
                "type"
            } else {
                "type"
            };
            Some((name, kind.to_string(), i + 1))
        } else if let Some(caps) = func_re.captures(trimmed) {
            Some((caps[1].to_string(), "function".into(), i + 1))
        } else if let Some(caps) = arrow_re.captures(trimmed) {
            let name = caps[1].to_string();
            let kind = if name.chars().next().map_or(false, |c| c.is_uppercase()) && trimmed.contains(".tsx") {
                "component"
            } else {
                "function"
            };
            Some((name, kind.to_string(), i + 1))
        } else if let Some(caps) = const_fn_re.captures(trimmed) {
            let name = caps[1].to_string();
            if name.chars().next().map_or(false, |c| c.is_uppercase()) {
                Some((name, "component".into(), i + 1))
            } else {
                Some((name, "function".into(), i + 1))
            }
        } else {
            None
        };

        if let Some((name, kind, line)) = sym {
            raw_symbols.push((name, kind, line));
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
        let body_hash = hash_str(&body);

        ParsedSymbol {
            name: name.clone(),
            kind: kind.clone(),
            line: *line,
            body_hash,
        }
    }).collect()
}

fn hash_str(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

pub fn extract_imports(src: &str) -> Vec<ParsedImport> {
    let mut imports = Vec::new();
    let import_re = Regex::new(r#"import\s+\{([^}]+)\}\s+from\s+['"](\.\.?/[^'"]+)['"]"#).unwrap();
    let import_default_re = Regex::new(r#"import\s+(\w+)\s+from\s+['"](\.\.?/[^'"]+)['"]"#).unwrap();
    let require_re = Regex::new(r#"require\(['"](\.\.?/[^'"]+)['"]\)"#).unwrap();

    for line in src.lines() {
        let trimmed = line.trim();
        if let Some(caps) = import_re.captures(trimmed) {
            let names: Vec<String> = caps[1].split(',').map(|s| {
                let s = s.trim();
                if let Some(pos) = s.find(" as ") { s[..pos].trim().to_string() }
                else { s.to_string() }
            }).filter(|s| !s.is_empty()).collect();
            imports.push(ParsedImport { source: caps[2].to_string(), names });
        } else if let Some(caps) = import_default_re.captures(trimmed) {
            imports.push(ParsedImport { source: caps[2].to_string(), names: vec![caps[1].to_string()] });
        } else if let Some(caps) = require_re.captures(trimmed) {
            imports.push(ParsedImport { source: caps[1].to_string(), names: vec![] });
        }
    }

    imports
}
