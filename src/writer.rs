use crate::config::CtxConfig;
use crate::rate;
use crate::state::CtxState;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

pub fn write_project_ctx(cfg: &CtxConfig, state: &CtxState) {
    let path = cfg.ctx_path.join("project.ctx");
    let content = render(cfg, state);
    std::fs::write(path, content).expect("failed to write project.ctx");
}

fn render(cfg: &CtxConfig, state: &CtxState) -> String {
    let mut out = String::with_capacity(8192);

    write_nest(&mut out, &state.nest);
    write_arch(&mut out, cfg, state);
    write_symbols(&mut out, state);
    write_log(&mut out, state);

    out
}

fn write_nest(out: &mut String, nest: &str) {
    out.push_str("---NEST\n");
    out.push_str(nest);
    out.push_str("\n---\n\n");
}

fn write_arch(out: &mut String, _cfg: &CtxConfig, state: &CtxState) {
    out.push_str("---ARCH\n");

    let mut modules: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for sym in state.symbols.values() {
        let parts: Vec<&str> = sym.fqn.split('/').collect();
        if parts.len() >= 2 {
            let module = parts[..parts.len() - 1].join("/");
            let entry = format!("{} [{}]", sym.name, sym.kind);
            modules.entry(module).or_default().insert(entry);
        }
    }

    let mut module_list: Vec<_> = modules.into_iter().collect();
    module_list.sort_by(|a, b| {
        let a_wph = rate::writes_per_hour(&state.rates, &a.0);
        let b_wph = rate::writes_per_hour(&state.rates, &b.0);
        let a_strength: f64 = state.symbols.values()
            .filter(|s| s.fqn.starts_with(&format!("{}/", a.0)))
            .map(|s| s.trail_strength).sum();
        let b_strength: f64 = state.symbols.values()
            .filter(|s| s.fqn.starts_with(&format!("{}/", b.0)))
            .map(|s| s.trail_strength).sum();
        b_wph.cmp(&a_wph)
            .then_with(|| b_strength.partial_cmp(&a_strength).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| a.0.cmp(&b.0))
    });

    for (module, symbols) in &module_list {
        let wph = rate::writes_per_hour(&state.rates, module);
        let activity = rate::activity_label(wph);
        let total_strength: f64 = state.symbols.values()
            .filter(|s| s.fqn.starts_with(&format!("{}/", module)))
            .map(|s| s.trail_strength).sum();
        let _ = writeln!(out, "{}/  {}w/hr {} heat:{:.1}", module, wph, activity, total_strength);

        for sym in symbols {
            let _ = writeln!(out, "  {}", sym);
        }
    }

    out.push_str("---\n\n");
}

fn write_symbols(out: &mut String, state: &CtxState) {
    out.push_str("---SYMBOLS\n");

    let mut sorted: Vec<_> = state.symbols.values().collect();
    sorted.sort_by(|a, b| {
        b.trail_strength.partial_cmp(&a.trail_strength)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.touch_count.cmp(&a.touch_count))
            .then_with(|| b.last_touched.cmp(&a.last_touched))
            .then_with(|| a.fqn.cmp(&b.fqn))
    });

    for sym in sorted {
        let dots = trail_dots(sym.trail_strength);
        let date = chrono::DateTime::from_timestamp(sym.last_touched, 0)
            .map(|d| d.format("%b%e").to_string().to_lowercase().replace(' ', ""))
            .unwrap_or_default();

        let _ = writeln!(out, "{} [{}] {} {}", sym.fqn, sym.kind, date, dots);

        if !sym.uses.is_empty() {
            let _ = writeln!(out, "  uses: {}", sym.uses.join(", "));
        }
        if !sym.used_by.is_empty() {
            let _ = writeln!(out, "  used by: {}", sym.used_by.join(", "));
        }

        let _ = writeln!(out, "  file: {}:{}", sym.file.display(), sym.line);
    }

    out.push_str("---\n\n");
}

fn write_log(out: &mut String, state: &CtxState) {
    out.push_str("---LOG\n");

    let start = if state.log.len() > 200 { state.log.len() - 200 } else { 0 };
    let mut recent: Vec<_> = state.log[start..].iter().collect();
    recent.reverse();
    for entry in &recent {
        let dt = chrono::DateTime::from_timestamp(entry.timestamp, 0)
            .map(|d| d.format("%m-%dT%H:%M").to_string())
            .unwrap_or_default();
        let dots = trail_dots(entry.trail_strength);
        let _ = writeln!(out, "{} {} {} {} {}", dt, entry.op, entry.path, entry.detail, dots);

        if !entry.rippled_to.is_empty() {
            let _ = writeln!(out, "  rippled to: {}", entry.rippled_to.join(", "));
        }
    }

    out.push_str("---\n");
}

fn trail_dots(strength: f64) -> String {
    let filled = (strength * 5.0).clamp(0.0, 5.0) as usize;
    let empty = 5 - filled;
    format!("{}{}", "●".repeat(filled), "○".repeat(empty))
}
