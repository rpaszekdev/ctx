use crate::config::CtxConfig;
use crate::engine;
use crate::rate;
use crate::state;
use crate::watcher;
use crate::writer;
use daemonize::Daemonize;
use notify::{RecursiveMode, Watcher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn start(cfg: CtxConfig) {
    let pid_path = cfg.ctx_path.join(".pid");
    let log_path = cfg.ctx_path.join("daemon.log");
    let stdout = std::fs::File::create(&log_path).expect("failed to create daemon.log");
    let stderr = stdout.try_clone().expect("failed to clone file handle");

    let daemon = Daemonize::new()
        .pid_file(&pid_path)
        .chown_pid_file(true)
        .working_directory(&cfg.project_root)
        .stdout(stdout)
        .stderr(stderr);

    match daemon.start() {
        Ok(_) => {
            eprintln!("ctx daemon forked (pid {})", std::process::id());
            run_loop(cfg);
        }
        Err(e) => {
            eprintln!("Failed to daemonize: {}", e);
            std::process::exit(1);
        }
    }
}

fn run_loop(cfg: CtxConfig) {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    setup_signal_handler(r);

    let mut st = state::load(&cfg);
    if st.symbols.is_empty() {
        engine::full_scan(&cfg, &mut st);
    }

    let (mut fs_watcher, raw_rx): (notify::RecommendedWatcher, _) = watcher::create_watcher(&cfg);
    fs_watcher
        .watch(&cfg.project_root, RecursiveMode::Recursive)
        .expect("failed to watch project root");

    let debounced_rx = watcher::watch_with_debounce(raw_rx, cfg.debounce_ms);

    let mut last_decay = Instant::now();
    let mut last_rate_cleanup = Instant::now();
    let mut last_save = Instant::now();
    let decay_interval = Duration::from_secs(cfg.decay_interval_secs);
    let rate_cleanup_interval = Duration::from_secs(300);

    while running.load(Ordering::Relaxed) {
        if let Ok(batch) = debounced_rx.recv_timeout(Duration::from_secs(1)) {
            for file in batch.into_iter().collect::<Vec<std::path::PathBuf>>() {
                engine::process_file_change(&cfg, &mut st, &file);
            }
            writer::write_project_ctx(&cfg, &st);

            if last_save.elapsed() > Duration::from_secs(1) {
                state::save(&cfg, &st);
                last_save = Instant::now();
            }
        }

        if last_decay.elapsed() >= decay_interval {
            st.decay_all(cfg.decay_factor);
            writer::write_project_ctx(&cfg, &st);
            state::save(&cfg, &st);
            last_decay = Instant::now();
        }

        if last_rate_cleanup.elapsed() >= rate_cleanup_interval {
            rate::cleanup(&mut st.rates);
            rate::cleanup_file_rates(&mut st.file_rates);
            last_rate_cleanup = Instant::now();
        }
    }

    state::save(&cfg, &st);
    writer::write_project_ctx(&cfg, &st);
}

fn setup_signal_handler(running: Arc<AtomicBool>) {
    use std::sync::atomic::AtomicBool as AB;
    static SIGNALED: AB = AB::new(false);

    unsafe {
        use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};
        extern "C" fn handler(_: i32) {
            SIGNALED.store(true, Ordering::Relaxed);
        }
        let action = SigAction::new(SigHandler::Handler(handler), SaFlags::empty(), SigSet::empty());
        let _ = sigaction(Signal::SIGTERM, &action);
        let _ = sigaction(Signal::SIGINT, &action);
    }

    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(200));
            if SIGNALED.load(Ordering::Relaxed) {
                running.store(false, Ordering::Relaxed);
                break;
            }
        }
    });
}
