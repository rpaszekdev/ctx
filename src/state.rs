use crate::config::CtxConfig;
use crate::graph::ImportGraph;
use crate::rate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CtxState {
    pub symbols: HashMap<String, TrackedSymbol>,
    pub graph: ImportGraph,
    pub rates: HashMap<String, Vec<i64>>,
    pub file_rates: HashMap<PathBuf, Vec<i64>>,
    pub log: Vec<LogEntry>,
    pub nest: String,
    #[serde(skip)]
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
    pub ripple_strength: f64,
    pub last_touched: i64,
    pub touch_count: u32,
    pub used_by: Vec<String>,
    pub uses: Vec<String>,
}

impl TrackedSymbol {
    pub fn total_heat(&self) -> f64 {
        self.trail_strength + self.ripple_strength
    }
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

fn log_heat_boost(current: f64, cap: f64) -> f64 {
    let remaining = cap - current;
    if remaining <= 0.0 {
        return current;
    }
    let boost = remaining * 0.3;
    (current + boost).min(cap)
}

impl CtxState {
    pub fn new() -> Self {
        Self {
            symbols: HashMap::new(),
            graph: ImportGraph::new(),
            rates: HashMap::new(),
            file_rates: HashMap::new(),
            log: Vec::new(),
            nest: default_nest(),
            file_cache: HashMap::new(),
        }
    }

    pub fn add_symbol(&mut self, fqn: String, name: String, kind: String, file: PathBuf, line: usize, initial_strength: f64) {
        let now = chrono::Utc::now().timestamp();
        self.symbols.insert(fqn.clone(), TrackedSymbol {
            name,
            kind,
            fqn,
            file,
            line,
            trail_strength: initial_strength,
            ripple_strength: 0.0,
            last_touched: now,
            touch_count: if initial_strength > 0.5 { 1 } else { 0 },
            used_by: Vec::new(),
            uses: Vec::new(),
        });
    }

    pub fn touch_symbol(&mut self, fqn: &str, line: usize) {
        if let Some(sym) = self.symbols.get_mut(fqn) {
            sym.trail_strength = log_heat_boost(sym.trail_strength, 5.0);
            sym.last_touched = chrono::Utc::now().timestamp();
            sym.touch_count += 1;
            sym.line = line;
        }
    }

    pub fn remove_symbol(&mut self, fqn: &str) {
        self.symbols.remove(fqn);
    }

    pub fn boost_ripple(&mut self, fqn: &str, boost: f64) {
        if let Some(sym) = self.symbols.get_mut(fqn) {
            sym.ripple_strength = (sym.ripple_strength + boost).min(5.0);
            sym.last_touched = chrono::Utc::now().timestamp();
        }
    }

    pub fn record_change(&mut self, entry: LogEntry) {
        self.log.push(entry);
    }

    pub fn record_file_write(&mut self, file: &Path) {
        rate::record_file_write(&mut self.file_rates, file);
    }

    pub fn record_module_write(&mut self, module: &str) {
        rate::record_write(&mut self.rates, module);
    }

    pub fn decay_all(&mut self, factor: f64) {
        for sym in self.symbols.values_mut() {
            sym.trail_strength = (sym.trail_strength * factor).max(0.0);
            sym.ripple_strength = (sym.ripple_strength * factor).max(0.0);
        }
    }

    pub fn update_file_cache(&mut self, file: PathBuf, content: Option<String>) {
        match content {
            Some(c) => { self.file_cache.insert(file, c); }
            None => { self.file_cache.remove(&file); }
        }
    }

    pub fn build_dependency_links(&mut self) {
        let graph_snapshot: Vec<(PathBuf, Vec<PathBuf>)> = self.graph.edges.iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let file_to_fqns: HashMap<PathBuf, Vec<String>> = {
            let mut map: HashMap<PathBuf, Vec<String>> = HashMap::new();
            for sym in self.symbols.values() {
                map.entry(sym.file.clone()).or_default().push(sym.fqn.clone());
            }
            map
        };

        for sym in self.symbols.values_mut() {
            sym.uses.clear();
            sym.used_by.clear();
        }

        for (file, imports) in &graph_snapshot {
            let source_fqns = file_to_fqns.get(file).cloned().unwrap_or_default();
            for imported_file in imports {
                let target_fqns = file_to_fqns.get(imported_file).cloned().unwrap_or_default();
                for src_fqn in &source_fqns {
                    for tgt_fqn in &target_fqns {
                        if let Some(sym) = self.symbols.get_mut(src_fqn) {
                            if !sym.uses.contains(tgt_fqn) {
                                sym.uses.push(tgt_fqn.clone());
                            }
                        }
                        if let Some(sym) = self.symbols.get_mut(tgt_fqn) {
                            if !sym.used_by.contains(src_fqn) {
                                sym.used_by.push(src_fqn.clone());
                            }
                        }
                    }
                }
            }
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
    let mut state = if path.exists() {
        let json = std::fs::read_to_string(path).expect("failed to read .state");
        serde_json::from_str(&json).unwrap_or_else(|_| CtxState::new())
    } else {
        CtxState::new()
    };
    state.nest = load_nest(cfg);
    state
}

pub fn load_nest(cfg: &CtxConfig) -> String {
    let nest_path = cfg.ctx_path.join("nest.md");
    if nest_path.exists() {
        std::fs::read_to_string(nest_path).unwrap_or_else(|_| default_nest())
    } else {
        default_nest()
    }
}

pub fn write_default_nest(cfg: &CtxConfig) {
    let nest_path = cfg.ctx_path.join("nest.md");
    std::fs::write(nest_path, default_nest()).expect("failed to write nest.md");
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
