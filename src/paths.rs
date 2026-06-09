use std::path::Path;

pub fn to_arch_path(rel: &Path) -> String {
    rel.to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("src/")
        .trim_end_matches(".ts").trim_end_matches(".tsx")
        .trim_end_matches(".js").trim_end_matches(".jsx")
        .trim_end_matches(".rs").trim_end_matches(".py")
        .trim_end_matches(".go")
        .to_string()
}

pub fn to_module(arch_path: &str) -> String {
    let parts: Vec<&str> = arch_path.split('/').collect();
    if parts.len() >= 2 { parts[..2].join("/") } else { parts.join("/") }
}
