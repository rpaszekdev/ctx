use crate::state::CtxState;
use std::path::PathBuf;

pub fn ripple(state: &mut CtxState, changed_file: &PathBuf, base_boost: f64, max_depth: u8) -> Vec<String> {
    let dependents = state.graph.transitive_dependents(changed_file, max_depth);
    let mut rippled = Vec::new();

    for (dep_file, depth) in dependents {
        let attenuation = 0.5_f64.powi(depth as i32);
        let boost = base_boost * attenuation;

        let fqns: Vec<String> = state.symbols.values()
            .filter(|s| s.file == dep_file)
            .map(|s| s.fqn.clone())
            .collect();

        for fqn in fqns {
            state.boost_ripple(&fqn, boost);
            rippled.push(fqn);
        }
    }

    rippled
}
