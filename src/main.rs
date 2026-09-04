mod config;
mod daemon;
mod differ;
mod engine;
mod graph;
mod parser;
mod paths;
mod rate;
mod resolver;
mod rippler;
mod state;
mod trails;
mod view;
mod walker;
mod watcher;
mod writer;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ctx", about = "Codebase context tracker — like git for meaning")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init,
    Start,
    Stop,
    Status,
    View,
    Log {
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    /// Show dependency graph for a file: what it imports and what imports it
    Deps {
        /// File path to inspect (relative to project root)
        file: PathBuf,
        #[arg(short, long, default_value = "2")]
        depth: u8,
        /// Output as JSON (easier for agents to parse)
        #[arg(long)]
        json: bool,
    },
    /// Brief context for a file — designed for hook injection into AI agents
    Brief {
        /// File path to get context for
        file: PathBuf,
        /// Max characters to output (default 500)
        #[arg(short, long, default_value = "500")]
        budget: usize,
    },
    /// Register a worker trail: what files were touched and by whom
    Touch {
        /// Files that were created or modified
        files: Vec<PathBuf>,
        #[arg(short, long)]
        worker: Option<String>,
    },
}

fn main() {
    env_logger::init();
    let cli = Cli::parse();
    let project_root = std::env::current_dir().expect("cannot determine cwd");

    match cli.command {
        Commands::Init => cmd_init(project_root),
        Commands::Start => cmd_start(project_root),
        Commands::Stop => cmd_stop(project_root),
        Commands::Status => cmd_status(project_root),
        Commands::View => cmd_view(project_root),
        Commands::Log { limit } => cmd_log(project_root, limit),
        Commands::Deps { file, depth, json } => cmd_deps(project_root, file, depth, json),
        Commands::Brief { file, budget } => cmd_brief(project_root, file, budget),
        Commands::Touch { files, worker } => cmd_touch(project_root, files, worker),
    }
}

fn cmd_init(root: PathBuf) {
    let cfg = config::CtxConfig::new(root.clone());
    let ctx_dir = root.join(".ctx");

    if ctx_dir.exists() {
        eprintln!(".ctx already initialized in {}", root.display());
        std::process::exit(1);
    }

    std::fs::create_dir_all(&ctx_dir).expect("failed to create .ctx/");
    std::fs::create_dir_all(ctx_dir.join("archive")).expect("failed to create .ctx/archive/");
    config::CtxConfig::write_default_config(&ctx_dir);
    state::write_default_nest(&cfg);

    let mut st = state::CtxState::new();
    let symbols_found = engine::full_scan(&cfg, &mut st);
    state::save(&cfg, &st);
    writer::write_project_ctx(&cfg, &st);

    println!("Initialized .ctx in {}", root.display());
    println!("Parsed {} symbols across project", symbols_found);
    println!("Generated .ctx/project.ctx");
}

fn cmd_start(root: PathBuf) {
    let cfg = config::CtxConfig::new(root.clone());
    let ctx_dir = root.join(".ctx");

    if !ctx_dir.exists() {
        eprintln!("Not a ctx project. Run `ctx init` first.");
        std::process::exit(1);
    }

    let pid_file = ctx_dir.join(".pid");
    if pid_file.exists() {
        if let Ok(pid_str) = std::fs::read_to_string(&pid_file) {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                if is_process_alive(pid) {
                    eprintln!("Daemon already running (pid {})", pid);
                    std::process::exit(1);
                }
            }
        }
        std::fs::remove_file(&pid_file).ok();
    }

    println!("Starting ctx daemon...");
    daemon::start(cfg);
    // After fork, parent continues here
    std::thread::sleep(std::time::Duration::from_millis(500));
    if let Ok(pid_str) = std::fs::read_to_string(ctx_dir.join(".pid")) {
        println!("Daemon running (pid {}). Watching for changes.", pid_str.trim());
    }
}

fn cmd_stop(root: PathBuf) {
    let pid_file = root.join(".ctx/.pid");
    if !pid_file.exists() {
        eprintln!("No running daemon found.");
        std::process::exit(1);
    }

    let pid_str = std::fs::read_to_string(&pid_file).expect("cannot read .pid");
    let pid: i32 = pid_str.trim().parse().expect("invalid pid");

    if is_process_alive(pid) {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGTERM,
        )
        .expect("failed to send SIGTERM");
        println!("Daemon stopped (pid {})", pid);
    } else {
        println!("Daemon was not running. Cleaning up.");
    }

    std::fs::remove_file(&pid_file).ok();
}

fn cmd_status(root: PathBuf) {
    let cfg = config::CtxConfig::new(root.clone());
    let ctx_dir = root.join(".ctx");

    if !ctx_dir.exists() {
        eprintln!("Not a ctx project. Run `ctx init` first.");
        std::process::exit(1);
    }

    let pid_file = ctx_dir.join(".pid");
    let running = if pid_file.exists() {
        std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|s| s.trim().parse::<i32>().ok())
            .map_or(false, is_process_alive)
    } else {
        false
    };

    let st = state::load(&cfg);
    let symbol_count = st.symbols.len();
    let log_count = st.log.len();
    let hot_count = st.symbols.values().filter(|s| s.total_heat() > 0.5).count();

    println!("project: {}", root.file_name().unwrap_or_default().to_string_lossy());
    println!("daemon:  {}", if running { "running" } else { "stopped" });
    println!("symbols: {} tracked ({} hot)", symbol_count, hot_count);
    println!("log:     {} entries", log_count);

    if !st.log.is_empty() {
        let last = st.log.last().unwrap();
        let dt = chrono::DateTime::from_timestamp(last.timestamp, 0)
            .map(|d| d.format("%m-%dT%H:%M").to_string())
            .unwrap_or_default();
        println!("last:    {}", dt);
    }
}

fn cmd_view(root: PathBuf) {
    let cfg = config::CtxConfig::new(root.clone());
    let ctx_dir = root.join(".ctx");

    if !ctx_dir.exists() {
        eprintln!("Not a ctx project. Run `ctx init` first.");
        std::process::exit(1);
    }

    let pid_file = ctx_dir.join(".pid");
    let running = if pid_file.exists() {
        std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|s| s.trim().parse::<i32>().ok())
            .map_or(false, is_process_alive)
    } else {
        false
    };

    let st = state::load(&cfg);
    view::render_view(&cfg, &st, running);
}

fn cmd_log(root: PathBuf, limit: usize) {
    let cfg = config::CtxConfig::new(root.clone());
    let ctx_dir = root.join(".ctx");

    if !ctx_dir.exists() {
        eprintln!("Not a ctx project. Run `ctx init` first.");
        std::process::exit(1);
    }

    let st = state::load(&cfg);
    let worker_trails = trails::load(&cfg);
    let start = if st.log.len() > limit { st.log.len() - limit } else { 0 };

    for entry in &st.log[start..] {
        let dt = chrono::DateTime::from_timestamp(entry.timestamp, 0)
            .map(|d| d.format("%m-%dT%H:%M").to_string())
            .unwrap_or_default();
        let dots = trail_dots(entry.trail_strength);
        let who = trails::attribute(&worker_trails, &entry.path, entry.timestamp)
            .map(|w| format!(" <{}>", w))
            .unwrap_or_default();
        println!("{} {} {} {} {}{}", dt, entry.op, entry.path, entry.detail, dots, who);
    }
}

use crate::view::trail_dots;

fn cmd_brief(root: PathBuf, file: PathBuf, budget: usize) {
    let cfg = config::CtxConfig::new(root.clone());
    let ctx_dir = root.join(".ctx");

    if !ctx_dir.exists() {
        return;
    }

    let st = state::load(&cfg);
    let abs_file = if file.is_absolute() { file } else { root.join(&file) };
    let rel_str = abs_file.strip_prefix(&root).unwrap_or(&abs_file).display().to_string();
    let now = chrono::Utc::now().timestamp();

    let last_write = rate::file_last_write(&st.file_rates, &abs_file);
    let secs_ago = last_write.map(|t| now - t);
    let writes_1m = rate::file_writes_last_n_secs(&st.file_rates, &abs_file, 60);
    let activity = rate::file_activity_label(writes_1m);

    let has_recent_activity = secs_ago.map_or(false, |s| s <= 300);

    let file_symbols: Vec<_> = st.symbols.values()
        .filter(|s| s.file == abs_file)
        .collect();

    let imports = st.graph.imports_of(&abs_file);
    let dependents = st.graph.dependents_of(&abs_file);

    let nearby_active: Vec<_> = imports.iter().chain(dependents.iter())
        .filter_map(|dep| {
            let dep_last = rate::file_last_write(&st.file_rates, dep)?;
            let dep_ago = now - dep_last;
            if dep_ago <= 300 {
                let dep_rel = dep.strip_prefix(&root).unwrap_or(dep).display().to_string();
                let dep_writes = rate::file_writes_last_n_secs(&st.file_rates, dep, 60);
                Some((dep_rel, dep_ago, dep_writes))
            } else {
                None
            }
        })
        .collect();

    if !has_recent_activity && nearby_active.is_empty() && file_symbols.is_empty() {
        return;
    }

    let mut out = String::new();
    use std::fmt::Write;

    let _ = writeln!(out, "[ctx] {}", rel_str);

    if has_recent_activity {
        let ago = secs_ago.unwrap_or(0);
        let ago_str = if ago < 60 { format!("{}s ago", ago) } else { format!("{}m ago", ago / 60) };
        let indicator = if activity == "editing" { " ⚡" } else { "" };
        let _ = writeln!(out, "  last_write:{}  writes_1m:{}  {}{}", ago_str, writes_1m, activity, indicator);
    }

    if !nearby_active.is_empty() {
        let _ = writeln!(out, "  nearby active:");
        for (dep, ago, w) in &nearby_active {
            if out.len() >= budget { break; }
            let ago_str = if *ago < 60 { format!("{}s", ago) } else { format!("{}m", ago / 60) };
            let _ = writeln!(out, "    {} {}ago writes_1m:{}", dep, ago_str, w);
        }
    }

    if !file_symbols.is_empty() && out.len() < budget {
        let _ = writeln!(out, "  symbols:");
        let mut sorted_syms: Vec<_> = file_symbols.iter().collect();
        sorted_syms.sort_by(|a, b| b.total_heat().partial_cmp(&a.total_heat()).unwrap_or(std::cmp::Ordering::Equal));
        for sym in sorted_syms {
            if out.len() >= budget { break; }
            let dots = trail_dots(sym.total_heat());
            let _ = writeln!(out, "    {} [{}] {} touches:{}", sym.name, sym.kind, dots, sym.touch_count);
        }
    }

    out.truncate(budget);
    print!("{}", out);
}

fn cmd_deps(root: PathBuf, file: PathBuf, depth: u8, json: bool) {
    let cfg = config::CtxConfig::new(root.clone());
    let ctx_dir = root.join(".ctx");

    if !ctx_dir.exists() {
        eprintln!("Not a ctx project. Run `ctx init` first.");
        std::process::exit(1);
    }

    let st = state::load(&cfg);
    let abs_file = if file.is_absolute() {
        file
    } else {
        root.join(&file)
    };

    let imports = st.graph.imports_of(&abs_file);
    let dependents = st.graph.transitive_dependents(&abs_file, depth);

    let file_symbols: Vec<_> = st.symbols.values()
        .filter(|s| s.file == abs_file)
        .collect();

    let rel_str = abs_file.strip_prefix(&root).unwrap_or(&abs_file).display().to_string();
    let recent_logs: Vec<_> = st.log.iter()
        .filter(|e| e.path.contains(&rel_str) || e.rippled_to.iter().any(|r| r.contains(&rel_str)))
        .rev()
        .take(5)
        .collect();

    if json {
        let output = serde_json::json!({
            "file": rel_str,
            "symbols": file_symbols.iter().map(|s| serde_json::json!({
                "name": s.name,
                "kind": s.kind,
                "heat": s.total_heat(),
                "direct_heat": s.trail_strength,
                "ripple_heat": s.ripple_strength,
                "touches": s.touch_count,
                "line": s.line,
            })).collect::<Vec<_>>(),
            "imports": imports.iter().map(|imp| {
                let rel = imp.strip_prefix(&root).unwrap_or(imp).display().to_string();
                let heat: f64 = st.symbols.values()
                    .filter(|s| s.file == *imp)
                    .map(|s| s.total_heat()).sum();
                serde_json::json!({"file": rel, "heat": heat})
            }).collect::<Vec<_>>(),
            "depended_on_by": dependents.iter().map(|(dep, d)| {
                let rel = dep.strip_prefix(&root).unwrap_or(dep).display().to_string();
                let heat: f64 = st.symbols.values()
                    .filter(|s| s.file == *dep)
                    .map(|s| s.total_heat()).sum();
                serde_json::json!({"file": rel, "depth": d, "heat": heat})
            }).collect::<Vec<_>>(),
            "recent_activity": recent_logs.iter().map(|e| {
                serde_json::json!({
                    "timestamp": e.timestamp,
                    "op": e.op,
                    "detail": e.detail,
                    "rippled_to": e.rippled_to,
                })
            }).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
        return;
    }

    println!("file: {}", rel_str);
    println!();

    if !file_symbols.is_empty() {
        println!("symbols:");
        for sym in &file_symbols {
            let dots = trail_dots(sym.total_heat());
            println!("  {} [{}] {} touches:{}", sym.name, sym.kind, dots, sym.touch_count);
        }
        println!();
    }

    if !imports.is_empty() {
        println!("imports ({}):", imports.len());
        for imp in &imports {
            let rel = imp.strip_prefix(&root).unwrap_or(imp);
            let heat: f64 = st.symbols.values()
                .filter(|s| s.file == *imp)
                .map(|s| s.total_heat())
                .sum();
            println!("  {} heat:{:.1}", rel.display(), heat);
        }
        println!();
    }

    if !dependents.is_empty() {
        println!("depended on by ({}):", dependents.len());
        for (dep, dep_depth) in &dependents {
            let rel = dep.strip_prefix(&root).unwrap_or(dep);
            let heat: f64 = st.symbols.values()
                .filter(|s| s.file == *dep)
                .map(|s| s.total_heat())
                .sum();
            let indent = "  ".repeat(*dep_depth as usize);
            println!("{}{}  depth:{} heat:{:.1}", indent, rel.display(), dep_depth, heat);
        }
        println!();
    }

    if !recent_logs.is_empty() {
        println!("recent activity:");
        for entry in &recent_logs {
            let dt = chrono::DateTime::from_timestamp(entry.timestamp, 0)
                .map(|d| d.format("%m-%dT%H:%M").to_string())
                .unwrap_or_default();
            println!("  {} {} {}", dt, entry.op, entry.detail);
        }
    }
}

fn cmd_touch(root: PathBuf, files: Vec<PathBuf>, worker: Option<String>) {
    let cfg = config::CtxConfig::new(root.clone());
    let ctx_dir = root.join(".ctx");

    if !ctx_dir.exists() {
        eprintln!("Not a ctx project. Run `ctx init` first.");
        std::process::exit(1);
    }

    let now = chrono::Utc::now();
    let worker_id = worker.unwrap_or_else(|| format!("worker-{}", now.timestamp_millis() % 10000));

    // Read-only: the daemon owns .state and already saw these writes via fs
    // events. All this command contributes is the identity behind them.
    let st = state::load(&cfg);
    let mut arch_paths = Vec::new();
    let mut touched_symbols = Vec::new();
    let mut rippled_all = Vec::new();

    for file in &files {
        let abs_file = if file.is_absolute() {
            file.clone()
        } else {
            root.join(file)
        };

        let arch = trails::arch_path_of(&abs_file, &root);
        if !arch_paths.contains(&arch) {
            arch_paths.push(arch);
        }

        let syms: Vec<String> = st.symbols.values()
            .filter(|s| s.file == abs_file)
            .map(|s| s.fqn.clone())
            .collect();
        touched_symbols.extend(syms);

        let dependents = st.graph.transitive_dependents(&abs_file, cfg.max_ripple_depth);
        for (dep, _) in &dependents {
            let rel = dep.strip_prefix(&root).unwrap_or(dep).display().to_string();
            if !rippled_all.contains(&rel) {
                rippled_all.push(rel);
            }
        }
    }

    let trail = trails::Trail {
        worker: worker_id.clone(),
        timestamp: now.timestamp(),
        arch_paths,
        files: files.iter().map(|f| f.display().to_string()).collect(),
    };

    if let Err(e) = trails::append(&cfg, &trail) {
        eprintln!("failed to record trail: {}", e);
        std::process::exit(1);
    }

    println!("trail: {}", worker_id);
    println!("  touched: {} files, {} symbols", files.len(), touched_symbols.len());
    if !rippled_all.is_empty() {
        println!("  rippled to: {}", rippled_all.join(", "));
    }
    println!("  saved: .ctx/trails.jsonl");
}

fn is_process_alive(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}
