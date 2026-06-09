pub mod typescript;
pub mod rust;
pub mod python;
pub mod go;

use std::path::Path;

#[derive(Debug, Clone)]
pub struct ParsedSymbol {
    pub name: String,
    pub kind: String,
    pub line: usize,
    pub body_hash: u64,
}

#[derive(Debug, Clone)]
pub struct ParsedImport {
    pub source: String,
    pub names: Vec<String>,
}

pub fn extract_symbols(src: &str, path: &Path) -> Vec<ParsedSymbol> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("ts") | Some("tsx") | Some("js") | Some("jsx") => typescript::extract_symbols(src),
        Some("rs") => rust::extract_symbols(src),
        Some("py") => python::extract_symbols(src),
        Some("go") => go::extract_symbols(src),
        _ => Vec::new(),
    }
}

pub fn extract_imports(src: &str, path: &Path) -> Vec<ParsedImport> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("ts") | Some("tsx") | Some("js") | Some("jsx") => typescript::extract_imports(src),
        _ => Vec::new(),
    }
}
