//! Managers' scenarios: named picks for a project's decisions, each tied to
//! the published version it was made on.
//!
//! They live in `<data>/scenarios/<project>.json`, outside the project's git
//! repository: they're managers' working notes, not changes to the model, and
//! the engineer's history stays about the model.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub id: String,
    pub name: String,
    /// The published version it was made on (`v2`).
    pub version: String,
    /// Decision id -> option id or number.
    pub decisions: Map<String, Value>,
    /// Seconds since 1970.
    pub created: u64,
}

pub struct Scenarios {
    root: PathBuf,
    /// One writer at a time: each project's scenarios are one file.
    lock: Mutex<()>,
}

impl Scenarios {
    pub fn new(root: PathBuf) -> Scenarios {
        Scenarios {
            root,
            lock: Mutex::new(()),
        }
    }

    fn file(&self, project: &str) -> PathBuf {
        self.root.join(format!("{project}.json"))
    }

    fn load(&self, project: &str) -> Result<Vec<Scenario>, String> {
        match fs::read_to_string(self.file(project)) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| format!("the scenarios of `{project}` can't be read: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn store(&self, project: &str, scenarios: &[Scenario]) -> Result<(), String> {
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(scenarios).map_err(|e| e.to_string())?;
        // Write then rename, so a crash never leaves half a file.
        let temp = self.root.join(format!(".{project}.json.tmp"));
        fs::write(&temp, text + "\n").map_err(|e| e.to_string())?;
        fs::rename(&temp, self.file(project)).map_err(|e| e.to_string())
    }

    /// Oldest first.
    pub fn list(&self, project: &str) -> Result<Vec<Scenario>, String> {
        let _guard = self.lock.lock().expect("scenarios lock");
        self.load(project)
    }

    pub fn get(&self, project: &str, id: &str) -> Result<Scenario, String> {
        self.list(project)?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| format!("no scenario `{id}`"))
    }

    pub fn add(
        &self,
        project: &str,
        name: &str,
        version: &str,
        decisions: Map<String, Value>,
    ) -> Result<Scenario, String> {
        let _guard = self.lock.lock().expect("scenarios lock");
        let mut scenarios = self.load(project)?;
        let next = scenarios
            .iter()
            .filter_map(|s| s.id.strip_prefix('s')?.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        let scenario = Scenario {
            id: format!("s{next}"),
            name: name.to_string(),
            version: version.to_string(),
            decisions,
            created: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        };
        scenarios.push(scenario.clone());
        self.store(project, &scenarios)?;
        Ok(scenario)
    }

    /// Renames a scenario and/or replaces its picks.
    pub fn update(
        &self,
        project: &str,
        id: &str,
        name: Option<&str>,
        decisions: Option<Map<String, Value>>,
    ) -> Result<Scenario, String> {
        let _guard = self.lock.lock().expect("scenarios lock");
        let mut scenarios = self.load(project)?;
        let scenario = scenarios
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| format!("no scenario `{id}`"))?;
        if let Some(name) = name {
            scenario.name = name.to_string();
        }
        if let Some(decisions) = decisions {
            scenario.decisions = decisions;
        }
        let updated = scenario.clone();
        self.store(project, &scenarios)?;
        Ok(updated)
    }

    pub fn remove(&self, project: &str, id: &str) -> Result<(), String> {
        let _guard = self.lock.lock().expect("scenarios lock");
        let mut scenarios = self.load(project)?;
        let before = scenarios.len();
        scenarios.retain(|s| s.id != id);
        if scenarios.len() == before {
            return Err(format!("no scenario `{id}`"));
        }
        self.store(project, &scenarios)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn add_update_remove() {
        let root =
            std::env::temp_dir().join(format!("outliers-sim-scenarios-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let store = Scenarios::new(root.clone());
        assert!(store.list("p").unwrap().is_empty());
        let picks = json!({ "inspectors": 2 }).as_object().unwrap().clone();
        let a = store
            .add("p", "Two inspectors", "v1", picks.clone())
            .unwrap();
        let b = store.add("p", "Other", "v1", Map::new()).unwrap();
        assert_eq!((a.id.as_str(), b.id.as_str()), ("s1", "s2"));
        store.update("p", "s2", Some("Renamed"), None).unwrap();
        assert_eq!(store.get("p", "s2").unwrap().name, "Renamed");
        store.remove("p", "s1").unwrap();
        assert!(store.remove("p", "s1").unwrap_err().contains("no scenario"));
        // Ids aren't reused while later ones exist.
        assert_eq!(store.add("p", "Third", "v1", picks).unwrap().id, "s3");
        assert_eq!(store.list("p").unwrap().len(), 2);
        fs::remove_dir_all(root).unwrap();
    }
}
