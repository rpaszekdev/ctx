use crate::config::CtxConfig;
use crate::graph::ImportGraph;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CtxState {
    pub symbols: HashMap<String, TrackedSymbol>,
    pub graph: ImportGraph,
    pub rates: HashMap<String, Vec<i64>>,
    pub log: Vec<LogEntry>,
    pub nest: String,
    pub file_cache: HashMap<PathBuf, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedSymbol {
    pub name: String,
    pub kind: String,
    pub fqn: String,
    pub file: PathBuf,
    pub line: usize,
    pub trail_strength: f64,
    pub last_touched: i64,
    pub touch_count: u32,
    pub used_by: Vec<String>,
    pub uses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: i64,
    pub op: String,
    pub path: String,
    pub detail: String,
    pub trail_strength: f64,
    pub rippled_to: Vec<String>,
}

impl CtxState {
    pub fn new() -> Self {
        Self {
            symbols: HashMap::new(),
            graph: ImportGraph::new(),
            rates: HashMap::new(),
            log: Vec::new(),
            nest: default_nest(),
            file_cache: HashMap::new(),
        }
    }
}

pub fn save(cfg: &CtxConfig, state: &CtxState) {
    let path = cfg.ctx_path.join(".state");
    let json = serde_json::to_string_pretty(state).expect("failed to serialize state");
    std::fs::write(path, json).expect("failed to write .state");
}

pub fn load(cfg: &CtxConfig) -> CtxState {
    let path = cfg.ctx_path.join(".state");
    if path.exists() {
        let json = std::fs::read_to_string(path).expect("failed to read .state");
        serde_json::from_str(&json).unwrap_or_else(|_| CtxState::new())
    } else {
        CtxState::new()
    }
}

fn default_nest() -> String {
    r#"structure:
  modules should be deep — few large modules, not many shallow ones
  screaming architecture — folders scream the domain, not the framework
  dependency rule — imports point inward, never outward

naming:
  one word per concept
  intention-revealing names
  if hard to name → redesign

functions:
  small, one thing
  0-2 arguments ideal
  command OR query, never both
  no side effects

state:
  prefer immutable
  return new copies, don't mutate

errors:
  crash early
  separate error handling from business logic
  never return null

changes:
  easier to change — every decision serves this
  boy scout rule — leave code cleaner than you found it"#
        .to_string()
}
