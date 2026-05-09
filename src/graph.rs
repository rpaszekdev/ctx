use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportGraph {
    pub edges: HashMap<PathBuf, Vec<PathBuf>>,
    pub reverse: HashMap<PathBuf, Vec<PathBuf>>,
}

impl ImportGraph {
    pub fn new() -> Self {
        Self {
            edges: HashMap::new(),
            reverse: HashMap::new(),
        }
    }

    pub fn set_imports(&mut self, file: PathBuf, imports: Vec<PathBuf>) {
        if let Some(old_imports) = self.edges.get(&file) {
            for old in old_imports {
                if let Some(rev) = self.reverse.get_mut(old) {
                    rev.retain(|f| f != &file);
                }
            }
        }
        for imp in &imports {
            self.reverse.entry(imp.clone()).or_default().push(file.clone());
        }
        self.edges.insert(file, imports);
    }

    pub fn dependents_of(&self, file: &PathBuf) -> Vec<PathBuf> {
        self.reverse.get(file).cloned().unwrap_or_default()
    }

    pub fn transitive_dependents(&self, file: &PathBuf, max_depth: u8) -> Vec<(PathBuf, u8)> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = vec![(file.clone(), 0u8)];

        while let Some((current, depth)) = queue.pop() {
            if depth > max_depth || visited.contains(&current) {
                continue;
            }
            visited.insert(current.clone());

            if depth > 0 {
                result.push((current.clone(), depth));
            }

            for dep in self.dependents_of(&current) {
                if !visited.contains(&dep) {
                    queue.push((dep, depth + 1));
                }
            }
        }

        result
    }

    pub fn imports_of(&self, file: &PathBuf) -> Vec<PathBuf> {
        self.edges.get(file).cloned().unwrap_or_default()
    }
}
