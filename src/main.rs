mod config;
mod daemon;
mod differ;
mod engine;
mod graph;
mod parser;
mod rate;
mod rippler;
mod state;
mod view;
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
    let hot_count = st.symbols.values().filter(|s| s.trail_strength > 0.5).count();

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
    view::render_view(&st, running);
}

fn cmd_log(root: PathBuf, limit: usize) {
    let cfg = config::CtxConfig::new(root.clone());
    let ctx_dir = root.join(".ctx");

    if !ctx_dir.exists() {
        eprintln!("Not a ctx project. Run `ctx init` first.");
        std::process::exit(1);
    }

    let st = state::load(&cfg);
    let start = if st.log.len() > limit { st.log.len() - limit } else { 0 };

    for entry in &st.log[start..] {
        let dt = chrono::DateTime::from_timestamp(entry.timestamp, 0)
            .map(|d| d.format("%m-%dT%H:%M").to_string())
            .unwrap_or_default();
        let dots = trail_dots(entry.trail_strength);
        println!("{} {} {} {} {}", dt, entry.op, entry.path, entry.detail, dots);
    }
}

fn trail_dots(strength: f64) -> String {
    let filled = (strength * 5.0).min(5.0) as usize;
    let empty = 5 - filled;
    format!("{}{}", "●".repeat(filled), "○".repeat(empty))
}

fn is_process_alive(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}
