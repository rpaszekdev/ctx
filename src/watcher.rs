use crate::config::CtxConfig;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

pub fn create_watcher(cfg: &CtxConfig) -> (RecommendedWatcher, mpsc::Receiver<PathBuf>) {
    let (tx, rx) = mpsc::channel();
    let cfg_clone = cfg.clone();

    let watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        if let Ok(event) = res {
            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                    for path in event.paths {
                        if cfg_clone.should_watch(&path) {
                            let _ = tx.send(path);
                        }
                    }
                }
                _ => {}
            }
        }
    })
    .expect("failed to create watcher");

    (watcher, rx)
}

pub fn watch_with_debounce(rx: mpsc::Receiver<PathBuf>, debounce_ms: u64) -> mpsc::Receiver<Vec<PathBuf>> {
    let (tx, out_rx) = mpsc::channel();

    std::thread::spawn(move || {
        loop {
            let mut batch = Vec::new();
            match rx.recv() {
                Ok(path) => batch.push(path),
                Err(_) => break,
            }

            std::thread::sleep(Duration::from_millis(debounce_ms));

            while let Ok(path) = rx.try_recv() {
                if !batch.contains(&path) {
                    batch.push(path);
                }
            }

            let _ = tx.send(batch);
        }
    });

    out_rx
}
