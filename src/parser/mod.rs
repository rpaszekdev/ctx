pub mod typescript;

use std::path::Path;

#[derive(Debug, Clone)]
pub struct ParsedSymbol {
    pub name: String,
    pub kind: String,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct ParsedImport {
    pub source: String,
    pub names: Vec<String>,
}

pub fn extract_symbols(src: &str, path: &Path) -> Vec<ParsedSymbol> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("ts") | Some("tsx") | Some("js") | Some("jsx") => typescript::extract_symbols(src),
        _ => typescript::extract_symbols(src),
    }
}

pub fn extract_imports(src: &str, path: &Path) -> Vec<ParsedImport> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("ts") | Some("tsx") | Some("js") | Some("jsx") => typescript::extract_imports(src),
        _ => typescript::extract_imports(src),
    }
}
