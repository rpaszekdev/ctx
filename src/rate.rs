use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn record_write(rates: &mut HashMap<String, Vec<i64>>, module: &str) {
    let now = chrono::Utc::now().timestamp();
    rates.entry(module.to_string()).or_default().push(now);
}

pub fn record_file_write(file_rates: &mut HashMap<PathBuf, Vec<i64>>, file: &Path) {
    let now = chrono::Utc::now().timestamp();
    file_rates.entry(file.to_path_buf()).or_default().push(now);
}

pub fn writes_per_hour(rates: &HashMap<String, Vec<i64>>, module: &str) -> usize {
    let now = chrono::Utc::now().timestamp();
    let one_hour_ago = now - 3600;
    rates.get(module).map_or(0, |timestamps| {
        timestamps.iter().filter(|&&t| t > one_hour_ago).count()
    })
}

pub fn file_writes_last_n_secs(file_rates: &HashMap<PathBuf, Vec<i64>>, file: &Path, secs: i64) -> usize {
    let now = chrono::Utc::now().timestamp();
    let cutoff = now - secs;
    file_rates.get(file).map_or(0, |timestamps| {
        timestamps.iter().filter(|&&t| t > cutoff).count()
    })
}

pub fn file_last_write(file_rates: &HashMap<PathBuf, Vec<i64>>, file: &Path) -> Option<i64> {
    file_rates.get(file).and_then(|ts| ts.last().copied())
}

pub fn file_activity_label(writes_1m: usize) -> &'static str {
    match writes_1m {
        0 => "idle",
        1..=2 => "recent",
        _ => "editing",
    }
}

pub fn cleanup(rates: &mut HashMap<String, Vec<i64>>) {
    let now = chrono::Utc::now().timestamp();
    let one_hour_ago = now - 3600;
    for timestamps in rates.values_mut() {
        timestamps.retain(|&t| t > one_hour_ago);
    }
    rates.retain(|_, v| !v.is_empty());
}

pub fn cleanup_file_rates(file_rates: &mut HashMap<PathBuf, Vec<i64>>) {
    let now = chrono::Utc::now().timestamp();
    let one_hour_ago = now - 3600;
    for timestamps in file_rates.values_mut() {
        timestamps.retain(|&t| t > one_hour_ago);
    }
    file_rates.retain(|_, v| !v.is_empty());
}

pub fn activity_label(wph: usize) -> &'static str {
    match wph {
        0 => "quiet",
        1..=3 => "calm",
        4..=8 => "active",
        _ => "storm",
    }
}
