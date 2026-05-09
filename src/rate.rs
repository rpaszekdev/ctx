use std::collections::HashMap;

pub fn record_write(rates: &mut HashMap<String, Vec<i64>>, module: &str) {
    let now = chrono::Utc::now().timestamp();
    rates.entry(module.to_string()).or_default().push(now);
}

pub fn writes_per_hour(rates: &HashMap<String, Vec<i64>>, module: &str) -> usize {
    let now = chrono::Utc::now().timestamp();
    let one_hour_ago = now - 3600;
    rates.get(module).map_or(0, |timestamps| {
        timestamps.iter().filter(|&&t| t > one_hour_ago).count()
    })
}

pub fn cleanup(rates: &mut HashMap<String, Vec<i64>>) {
    let now = chrono::Utc::now().timestamp();
    let one_hour_ago = now - 3600;
    for timestamps in rates.values_mut() {
        timestamps.retain(|&t| t > one_hour_ago);
    }
    rates.retain(|_, v| !v.is_empty());
}

pub fn activity_label(wph: usize) -> &'static str {
    match wph {
        0 => "quiet",
        1..=3 => "calm",
        4..=8 => "active",
        _ => "storm",
    }
}
