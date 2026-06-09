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
    write_active(&mut out, cfg, state);
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

fn write_active(out: &mut String, cfg: &CtxConfig, state: &CtxState) {
    let now = chrono::Utc::now().timestamp();
    let mut active_files: Vec<_> = state.file_rates.iter()
        .filter_map(|(file, timestamps)| {
            let writes_1m = timestamps.iter().filter(|&&t| t > now - 60).count();
            let last_write = timestamps.last().copied().unwrap_or(0);
            let secs_ago = now - last_write;
            if secs_ago <= 300 {
                let rel = file.strip_prefix(&cfg.project_root).unwrap_or(file);
                let label = rate::file_activity_label(writes_1m);
                Some((rel.to_path_buf(), secs_ago, writes_1m, label))
            } else {
                None
            }
        })
        .collect();

    if active_files.is_empty() {
        return;
    }

    active_files.sort_by_key(|(_, secs_ago, _, _)| *secs_ago);

    out.push_str("---ACTIVE\n");
    for (file, secs_ago, writes_1m, label) in &active_files {
        let ago = if *secs_ago < 60 {
            format!("{}s ago", secs_ago)
        } else {
            format!("{}m ago", secs_ago / 60)
        };
        let indicator = if *label == "editing" { " ⚡" } else { "" };
        let _ = std::fmt::Write::write_fmt(out, format_args!(
            "{}  last_write:{}  writes_1m:{}  {}{}\n",
            file.display(), ago, writes_1m, label, indicator
        ));
    }
    out.push_str("---\n\n");
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
        let a_heat: f64 = state.symbols.values()
            .filter(|s| s.fqn.starts_with(&format!("{}/", a.0)))
            .map(|s| s.total_heat()).sum();
        let b_heat: f64 = state.symbols.values()
            .filter(|s| s.fqn.starts_with(&format!("{}/", b.0)))
            .map(|s| s.total_heat()).sum();
        b_wph.cmp(&a_wph)
            .then_with(|| b_heat.partial_cmp(&a_heat).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| a.0.cmp(&b.0))
    });

    for (module, symbols) in &module_list {
        let wph = rate::writes_per_hour(&state.rates, module);
        let activity = rate::activity_label(wph);
        let total_heat: f64 = state.symbols.values()
            .filter(|s| s.fqn.starts_with(&format!("{}/", module)))
            .map(|s| s.total_heat()).sum();
        let _ = writeln!(out, "{}/  {}w/hr {} heat:{:.1}", module, wph, activity, total_heat);

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
        b.total_heat().partial_cmp(&a.total_heat())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.touch_count.cmp(&a.touch_count))
            .then_with(|| b.last_touched.cmp(&a.last_touched))
            .then_with(|| a.fqn.cmp(&b.fqn))
    });

    for sym in sorted {
        let direct = trail_dots(sym.trail_strength);
        let ripple = if sym.ripple_strength > 0.1 {
            format!(" ripple:{:.1}", sym.ripple_strength)
        } else {
            String::new()
        };
        let date = chrono::DateTime::from_timestamp(sym.last_touched, 0)
            .map(|d| d.format("%b%e").to_string().to_lowercase().replace(' ', ""))
            .unwrap_or_default();

        let _ = writeln!(out, "{} [{}] {} {}{}", sym.fqn, sym.kind, date, direct, ripple);

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
