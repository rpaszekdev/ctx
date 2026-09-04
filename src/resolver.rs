//! Import specifier → file path.
//!
//! Only resolved imports become graph edges, and only graph edges let heat
//! ripple — so an unsupported language here means that language contributes
//! nothing but a decayed touch count. Unresolvable specifiers (stdlib, external
//! packages) are dropped silently: they have no file in this project, so there
//! is nothing to ripple to.

use crate::parser;
use std::path::{Path, PathBuf};

const TS_EXTENSIONS: [&str; 4] = ["ts", "tsx", "js", "jsx"];

pub fn resolve_imports(
    imports: &[parser::ParsedImport],
    from_file: &Path,
    project_root: &Path,
) -> Vec<PathBuf> {
    let lang = from_file.extension().and_then(|e| e.to_str()).unwrap_or("");
    let mut resolved = Vec::new();

    for imp in imports {
        let hit = match lang {
            "ts" | "tsx" | "js" | "jsx" => resolve_ts(imp, from_file, project_root),
            "py" => resolve_python(imp, from_file, project_root),
            "rs" => resolve_rust(imp, from_file, project_root),
            _ => None,
        };
        if let Some(path) = hit {
            let path = normalize(&path);
            if !resolved.contains(&path) {
                resolved.push(path);
            }
        }
    }

    resolved
}

// ── TypeScript / JavaScript ─────────────────────────────────────────────

fn resolve_ts(imp: &parser::ParsedImport, from_file: &Path, project_root: &Path) -> Option<PathBuf> {
    let dir = from_file.parent().unwrap_or(project_root);

    if imp.source.starts_with('.') {
        return probe_ts(&dir.join(&imp.source));
    }

    // Bare specifier: an alias like `@/lib/supa` if tsconfig maps it, otherwise
    // a node_modules package we do not track.
    alias_bases(from_file, project_root, &imp.source)
        .into_iter()
        .find_map(|base| probe_ts(&base))
}

/// `foo` → `foo.ts`, `foo.tsx`, … then `foo/index.ts`, …
fn probe_ts(base: &Path) -> Option<PathBuf> {
    // Specifiers are usually extensionless, but `./App.tsx` is legal and common
    // under bundlers — check the literal path before appending anything.
    if base.extension().is_some() && base.is_file() {
        return Some(base.to_path_buf());
    }

    for ext in TS_EXTENSIONS {
        let candidate = with_appended_extension(base, ext);
        if candidate.is_file() {
            return Some(candidate);
        }
        let index = base.join(format!("index.{}", ext));
        if index.is_file() {
            return Some(index);
        }
    }
    None
}

/// Candidate on-disk bases for a bare specifier, from the nearest tsconfig's
/// `paths` and `baseUrl`.
///
/// Walks up from the file rather than assuming one tsconfig at the root — a repo
/// with several apps has several, each with its own `@/*` meaning a different
/// directory. The nearest one wins, which is also how tsc resolves.
fn alias_bases(from_file: &Path, project_root: &Path, specifier: &str) -> Vec<PathBuf> {
    let mut bases = Vec::new();
    let mut dir = from_file.parent();

    while let Some(current) = dir {
        let tsconfig = current.join("tsconfig.json");
        if tsconfig.is_file() {
            if let Some(config) = read_tsconfig(&tsconfig) {
                for (pattern, targets) in &config.paths {
                    if let Some(tail) = match_alias(pattern, specifier) {
                        for target in targets {
                            let target = target.trim_end_matches("/*").trim_end_matches('*');
                            let base = current.join(&config.base_url).join(target);
                            bases.push(if tail.is_empty() { base } else { base.join(&tail) });
                        }
                    }
                }
                if !config.base_url.is_empty() {
                    bases.push(current.join(&config.base_url).join(specifier));
                }
            }
            break;
        }
        if current == project_root {
            break;
        }
        dir = current.parent();
    }

    bases
}

/// `@/*` vs `@/lib/supa` → `lib/supa`. Exact patterns yield an empty tail.
fn match_alias(pattern: &str, specifier: &str) -> Option<String> {
    match pattern.strip_suffix('*') {
        Some(prefix) => specifier.strip_prefix(prefix).map(|s| s.to_string()),
        None => (pattern == specifier).then(String::new),
    }
}

struct TsConfig {
    base_url: String,
    paths: Vec<(String, Vec<String>)>,
}

/// tsconfig.json is JSON with comments and trailing commas, which `serde_json`
/// rejects — strip both before parsing rather than adding a jsonc dependency.
fn read_tsconfig(path: &Path) -> Option<TsConfig> {
    let raw = std::fs::read_to_string(path).ok()?;
    let cleaned = strip_jsonc(&raw);
    let json: serde_json::Value = serde_json::from_str(&cleaned).ok()?;
    let options = json.get("compilerOptions")?;

    let base_url = options
        .get("baseUrl")
        .and_then(|v| v.as_str())
        .unwrap_or(".")
        .to_string();

    let paths = options
        .get("paths")
        .and_then(|v| v.as_object())
        .map(|map| {
            map.iter()
                .map(|(pattern, targets)| {
                    let targets = targets
                        .as_array()
                        .map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect())
                        .unwrap_or_default();
                    (pattern.clone(), targets)
                })
                .collect()
        })
        .unwrap_or_default();

    Some(TsConfig { base_url, paths })
}

fn strip_jsonc(raw: &str) -> String {
    let no_block = regex::Regex::new(r"(?s)/\*.*?\*/").unwrap().replace_all(raw, "");
    let no_line = regex::Regex::new(r"(?m)^\s*//.*$").unwrap().replace_all(&no_block, "");
    regex::Regex::new(r",(\s*[}\]])").unwrap().replace_all(&no_line, "$1").to_string()
}

// ── Python ──────────────────────────────────────────────────────────────

/// `from .a import b` climbs from the file's own directory; `import a.b` is
/// searched against every ancestor up to the project root, since Python's
/// `sys.path` normally includes both the entry script's directory and the root.
fn resolve_python(
    imp: &parser::ParsedImport,
    from_file: &Path,
    project_root: &Path,
) -> Option<PathBuf> {
    let dir = from_file.parent().unwrap_or(project_root);
    let leading_dots = imp.source.chars().take_while(|c| *c == '.').count();
    let module = imp.source.trim_start_matches('.');

    let search_roots: Vec<PathBuf> = if leading_dots > 0 {
        let mut base = dir.to_path_buf();
        for _ in 1..leading_dots {
            base = base.parent()?.to_path_buf();
        }
        vec![base]
    } else {
        ancestors_within(dir, project_root)
    };

    let segments: Vec<&str> = module.split('.').filter(|s| !s.is_empty()).collect();

    for root in &search_roots {
        let base = segments.iter().fold(root.clone(), |acc, seg| acc.join(seg));

        // `from a.b import c` — c may itself be a module (a/b/c.py) before it is
        // a symbol inside a/b.py.
        for name in &imp.names {
            if let Some(hit) = probe_python(&base.join(name)) {
                return Some(hit);
            }
        }
        if let Some(hit) = probe_python(&base) {
            return Some(hit);
        }
    }

    None
}

fn probe_python(base: &Path) -> Option<PathBuf> {
    let module = with_appended_extension(base, "py");
    if module.is_file() {
        return Some(module);
    }
    let package = base.join("__init__.py");
    package.is_file().then_some(package)
}

/// The directory and each parent up to (and including) the project root.
fn ancestors_within(dir: &Path, project_root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut current = Some(dir);

    while let Some(path) = current {
        roots.push(path.to_path_buf());
        if path == project_root {
            break;
        }
        current = path.parent();
    }

    if !roots.iter().any(|r| r == project_root) {
        roots.push(project_root.to_path_buf());
    }
    roots
}

// ── Rust ────────────────────────────────────────────────────────────────

fn resolve_rust(
    imp: &parser::ParsedImport,
    from_file: &Path,
    project_root: &Path,
) -> Option<PathBuf> {
    let dir = from_file.parent().unwrap_or(project_root);
    let segments: Vec<&str> = imp.source.split("::").filter(|s| !s.is_empty()).collect();
    let first = *segments.first()?;

    let (base, rest) = match first {
        "crate" => (project_root.join("src"), &segments[1..]),
        "self" => (dir.to_path_buf(), &segments[1..]),
        "super" => (dir.parent()?.to_path_buf(), &segments[1..]),
        // `mod x;` and sibling paths resolve next to the current file; anything
        // else is an external crate and resolves to nothing.
        _ => (dir.to_path_buf(), &segments[..]),
    };

    // Trailing segments may be types or functions rather than modules, so try
    // the longest path first and shorten until something exists on disk.
    for take in (0..=rest.len()).rev() {
        let candidate = rest[..take].iter().fold(base.clone(), |acc, seg| acc.join(seg));
        if candidate == base && take == 0 {
            continue;
        }
        if let Some(hit) = probe_rust(&candidate) {
            return Some(hit);
        }
    }

    None
}

fn probe_rust(base: &Path) -> Option<PathBuf> {
    let module = with_appended_extension(base, "rs");
    if module.is_file() {
        return Some(module);
    }
    let dir_module = base.join("mod.rs");
    dir_module.is_file().then_some(dir_module)
}

// ── Shared ──────────────────────────────────────────────────────────────

/// Collapses `.` and `..` lexically.
///
/// `dir.join("./Foo")` keeps the `.` as a real path component, so the result is
/// byte-unequal to the same file as the walker records it — and the import graph
/// is keyed by path. Without this, every relative import becomes an edge that
/// matches nothing and heat has nowhere to ripple.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Appends rather than replaces: `Path::with_extension` would turn `market.v2`
/// into `market.py`, losing a real path segment.
fn with_appended_extension(base: &Path, ext: &str) -> PathBuf {
    let mut name = base.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(ext);
    base.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imp(source: &str, names: &[&str]) -> parser::ParsedImport {
        parser::ParsedImport {
            source: source.to_string(),
            names: names.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(root: &Path, rel: &str) -> PathBuf {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "").unwrap();
        path
    }

    #[test]
    fn python_absolute_import_resolves_against_the_project_root() {
        let root = scratch("ctx-res-py-abs");
        let target = write(&root, "app/market.py");
        let from = write(&root, "app/api.py");

        let got = resolve_imports(&[imp("app.market", &[])], &from, &root);
        assert_eq!(got, vec![target]);
    }

    #[test]
    fn python_sibling_import_resolves_next_to_the_file() {
        let root = scratch("ctx-res-py-sibling");
        let target = write(&root, "agoda/admin.py");
        let from = write(&root, "agoda/app.py");

        let got = resolve_imports(&[imp("admin", &[])], &from, &root);
        assert_eq!(got, vec![target]);
    }

    #[test]
    fn python_relative_import_climbs_one_level_per_extra_dot() {
        let root = scratch("ctx-res-py-rel");
        let target = write(&root, "pkg/shared.py");
        let from = write(&root, "pkg/sub/deep.py");

        let got = resolve_imports(&[imp("..shared", &[])], &from, &root);
        assert_eq!(got, vec![target]);
    }

    #[test]
    fn python_from_import_prefers_a_submodule_over_its_parent() {
        let root = scratch("ctx-res-py-submodule");
        write(&root, "pkg/__init__.py");
        let target = write(&root, "pkg/child.py");
        let from = write(&root, "main.py");

        let got = resolve_imports(&[imp("pkg", &["child"])], &from, &root);
        assert_eq!(got, vec![target]);
    }

    #[test]
    fn python_package_resolves_to_its_init() {
        let root = scratch("ctx-res-py-init");
        let target = write(&root, "pkg/__init__.py");
        let from = write(&root, "main.py");

        let got = resolve_imports(&[imp("pkg", &[])], &from, &root);
        assert_eq!(got, vec![target]);
    }

    #[test]
    fn python_stdlib_imports_resolve_to_nothing() {
        let root = scratch("ctx-res-py-stdlib");
        let from = write(&root, "main.py");

        let got = resolve_imports(&[imp("os", &[]), imp("flask", &["Flask"])], &from, &root);
        assert!(got.is_empty());
    }

    #[test]
    fn ts_alias_resolves_through_the_nearest_tsconfig() {
        let root = scratch("ctx-res-ts-alias");
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::write(
            root.join("app/tsconfig.json"),
            r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "@/*": ["src/*"] } } }"#,
        )
        .unwrap();
        let target = write(&root, "app/src/lib/supa.ts");
        let from = write(&root, "app/src/screens/Page.tsx");

        let got = resolve_imports(&[imp("@/lib/supa", &[])], &from, &root);
        assert_eq!(got, vec![target]);
    }

    #[test]
    fn ts_alias_uses_the_nearest_tsconfig_not_the_outermost() {
        let root = scratch("ctx-res-ts-nearest");
        std::fs::write(
            root.join("tsconfig.json"),
            r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "@/*": ["outer/*"] } } }"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::write(
            root.join("app/tsconfig.json"),
            r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "@/*": ["src/*"] } } }"#,
        )
        .unwrap();
        write(&root, "outer/thing.ts");
        let inner = write(&root, "app/src/thing.ts");
        let from = write(&root, "app/src/Page.tsx");

        let got = resolve_imports(&[imp("@/thing", &[])], &from, &root);
        assert_eq!(got, vec![inner]);
    }

    #[test]
    fn tsconfig_with_comments_and_trailing_commas_still_parses() {
        let root = scratch("ctx-res-ts-jsonc");
        std::fs::write(
            root.join("tsconfig.json"),
            "{\n // the app\n \"compilerOptions\": {\n \"baseUrl\": \".\",\n \"paths\": { \"@/*\": [\"src/*\"], },\n },\n}",
        )
        .unwrap();
        let target = write(&root, "src/thing.ts");
        let from = write(&root, "src/Page.tsx");

        let got = resolve_imports(&[imp("@/thing", &[])], &from, &root);
        assert_eq!(got, vec![target]);
    }

    #[test]
    fn a_relative_import_resolves_to_the_same_path_the_walker_records() {
        let root = scratch("ctx-res-normalize");
        let target = write(&root, "src/lib/api.ts");
        let from = write(&root, "src/lib/caller.ts");

        let got = resolve_imports(&[imp("./api", &[])], &from, &root);

        // Byte equality is what matters — the import graph is keyed by path, so
        // a stray "." component silently produces an edge that matches nothing.
        assert_eq!(got, vec![target.clone()]);
        assert!(!got[0].to_string_lossy().contains("/./"));

        let parent_hop = resolve_imports(&[imp("../lib/api", &[])], &from, &root);
        assert_eq!(parent_hop, vec![target]);
    }

    #[test]
    fn ts_relative_import_still_resolves() {
        let root = scratch("ctx-res-ts-rel");
        let target = write(&root, "src/lib/api.ts");
        let from = write(&root, "src/lib/caller.ts");

        let got = resolve_imports(&[imp("./api", &[])], &from, &root);
        assert_eq!(got, vec![target]);
    }

    #[test]
    fn ts_directory_import_resolves_to_index() {
        let root = scratch("ctx-res-ts-index");
        let target = write(&root, "src/lib/index.ts");
        let from = write(&root, "src/caller.ts");

        let got = resolve_imports(&[imp("./lib", &[])], &from, &root);
        assert_eq!(got, vec![target]);
    }

    #[test]
    fn rust_crate_path_resolves_under_src() {
        let root = scratch("ctx-res-rs-crate");
        let target = write(&root, "src/state.rs");
        let from = write(&root, "src/engine.rs");

        let got = resolve_imports(&[imp("crate::state::CtxState", &[])], &from, &root);
        assert_eq!(got, vec![target]);
    }

    #[test]
    fn rust_module_directory_resolves_to_mod_rs() {
        let root = scratch("ctx-res-rs-mod");
        let target = write(&root, "src/parser/mod.rs");
        let from = write(&root, "src/engine.rs");

        let got = resolve_imports(&[imp("crate::parser", &[])], &from, &root);
        assert_eq!(got, vec![target]);
    }

    #[test]
    fn rust_external_crate_resolves_to_nothing() {
        let root = scratch("ctx-res-rs-extern");
        let from = write(&root, "src/engine.rs");

        let got = resolve_imports(&[imp("serde::Serialize", &[])], &from, &root);
        assert!(got.is_empty());
    }

    #[test]
    fn ts_specifier_that_already_has_an_extension_resolves() {
        let root = scratch("ctx-res-ts-explicit-ext");
        let target = write(&root, "src/App.tsx");
        let from = write(&root, "src/main.tsx");

        // `import App from "./App.tsx"` — appending another extension would look
        // for App.tsx.ts and find nothing.
        let got = resolve_imports(&[imp("./App.tsx", &[])], &from, &root);
        assert_eq!(got, vec![target]);
    }
}
