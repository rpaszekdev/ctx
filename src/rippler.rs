use crate::state::CtxState;
use std::path::PathBuf;

pub fn ripple(state: &mut CtxState, changed_file: &PathBuf, base_boost: f64, max_depth: u8) -> Vec<String> {
    let dependents = state.graph.transitive_dependents(changed_file, max_depth);
    let mut rippled = Vec::new();

    for (dep_file, depth) in dependents {
        let attenuation = 0.5_f64.powi(depth as i32);
        let boost = base_boost * attenuation;

        for sym in state.symbols.values_mut() {
            if sym.file == dep_file {
                sym.trail_strength = (sym.trail_strength + boost).min(5.0);
                sym.last_touched = chrono::Utc::now().timestamp();
                rippled.push(sym.fqn.clone());
            }
        }
    }

    rippled
}
