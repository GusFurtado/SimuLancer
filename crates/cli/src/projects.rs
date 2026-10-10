//! Projects: one folder per project, each a local git repository.
//!
//! ```text
//! projects/<id>/
//!   project.yaml        name, description, which file is the model
//!   model.yaml          the model
//!   model.layout.yaml   its layout (optional)
//! ```
//!
//! `project.yaml` also says what managers may change and what they read
//! (`decisions` and `results`, see the `decisions` module).
//!
//! The engineer saves versions (git commits) and publishes them (tags `v1`,
//! `v2`, ...). Managers only see published versions, read straight from git
//! at their tag, so later edits never change what they're looking at.
//! Git is run as a command; the engineer never sees it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const PROJECT_FORMAT: &str = "outliers-sim-project/0.1";
pub const PROJECT_FILE: &str = "project.yaml";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectFile {
    pub format: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The model file, relative to the project folder.
    pub model: String,
    /// What managers may change: decision id -> decision.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub decisions: Map<String, Value>,
    /// What managers read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<Value>,
}

impl ProjectFile {
    /// As `project.yaml` text.
    pub fn to_text(&self) -> String {
        let tree = serde_json::to_value(self).expect("a project file is plain data");
        outliers_sim_model::to_yaml(&tree)
    }

    pub fn layout_file(&self) -> String {
        let stem = self.model.strip_suffix(".yaml").unwrap_or(&self.model);
        format!("{stem}.layout.yaml")
    }
}

/// A published version: a `vN` tag.
#[derive(Debug, Clone, Serialize)]
pub struct Version {
    pub name: String,
    pub number: u32,
    pub date: String,
    pub message: String,
    pub commit: String,
}

/// A saved version: a commit.
#[derive(Debug, Clone, Serialize)]
pub struct Commit {
    pub id: String,
    pub short: String,
    pub date: String,
    pub message: String,
    /// The `vN` tag on this commit, if it was published.
    pub published: Option<String>,
}

pub struct Projects {
    pub root: PathBuf,
}

/// Project ids are folder names: lowercase letters, digits, `-` and `_`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

impl Projects {
    pub fn new(root: PathBuf) -> Projects {
        Projects { root }
    }

    pub fn dir(&self, id: &str) -> Result<PathBuf, String> {
        if !valid_id(id) {
            return Err(format!(
                "`{id}` is not a project id (lowercase letters, digits, `-`, `_`)"
            ));
        }
        let dir = self.root.join(id);
        if dir.join(PROJECT_FILE).is_file() {
            Ok(dir)
        } else {
            Err(format!("no project `{id}`"))
        }
    }

    /// Project ids, sorted.
    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|id| valid_id(id) && self.root.join(id).join(PROJECT_FILE).is_file())
            .collect();
        ids.sort();
        ids
    }

    /// Creates a project from files and saves them as the first version.
    pub fn create(
        &self,
        id: &str,
        files: &[(String, String)],
        message: &str,
    ) -> Result<(), String> {
        if !valid_id(id) {
            return Err(format!(
                "`{id}` is not a project id (lowercase letters, digits, `-`, `_`)"
            ));
        }
        let dir = self.root.join(id);
        if dir.exists() {
            return Err(format!("a project `{id}` already exists"));
        }
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let result = (|| {
            git(&dir, &["init", "-q", "-b", "main"])?;
            for (name, content) in files {
                fs::write(dir.join(name), content).map_err(|e| e.to_string())?;
            }
            git(&dir, &["add", "-A"])?;
            git(&dir, &["commit", "-q", "-m", message])?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&dir);
        }
        result
    }

    pub fn project_file(&self, id: &str, version: Option<&str>) -> Result<ProjectFile, String> {
        let text = self.read(id, version, PROJECT_FILE)?;
        let file: ProjectFile =
            serde_saphyr::from_str(&text).map_err(|e| format!("{id}/{PROJECT_FILE}: {e}"))?;
        if file.format != PROJECT_FORMAT {
            return Err(format!(
                "{id}/{PROJECT_FILE}: unsupported format `{}` (this version reads `{PROJECT_FORMAT}`)",
                file.format
            ));
        }
        Ok(file)
    }

    /// A file's text: from the working folder (`version` = None) or as
    /// published (`Some("v3")`). `Ok(None)` if the file doesn't exist there.
    pub fn read_optional(
        &self,
        id: &str,
        version: Option<&str>,
        file: &str,
    ) -> Result<Option<String>, String> {
        let dir = self.dir(id)?;
        match version {
            None => match fs::read_to_string(dir.join(file)) {
                Ok(text) => Ok(Some(text)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.to_string()),
            },
            Some(v) => {
                let tag = self.version_tag(id, v)?;
                match git(&dir, &["show", &format!("{tag}:{file}")]) {
                    Ok(text) => Ok(Some(text)),
                    Err(_) => Ok(None),
                }
            }
        }
    }

    pub fn read(&self, id: &str, version: Option<&str>, file: &str) -> Result<String, String> {
        self.read_optional(id, version, file)?
            .ok_or_else(|| format!("project `{id}` has no `{file}`"))
    }

    /// Writes files and saves them as a new version (a commit).
    pub fn save(&self, id: &str, files: &[(&str, &str)], message: &str) -> Result<Commit, String> {
        let dir = self.dir(id)?;
        for (name, content) in files {
            fs::write(dir.join(name), content).map_err(|e| e.to_string())?;
        }
        git(&dir, &["add", "-A"])?;
        if git(&dir, &["status", "--porcelain"])?.trim().is_empty() {
            return Err("nothing changed since the last saved version".into());
        }
        let message = if message.trim().is_empty() {
            "Saved from the builder"
        } else {
            message
        };
        git(&dir, &["commit", "-q", "-m", message])?;
        Ok(self.history(id)?.remove(0))
    }

    /// Publishes the last saved version as `v<next>`. Without a message, the
    /// version is described by the saved version's own message, which is
    /// what the engineer wrote about the change.
    pub fn publish(&self, id: &str, message: &str) -> Result<Version, String> {
        let dir = self.dir(id)?;
        if !git(&dir, &["status", "--porcelain"])?.trim().is_empty() {
            return Err("the project has unsaved changes on disk; save them first".into());
        }
        let versions = self.versions(id)?;
        let head = git(&dir, &["rev-parse", "HEAD"])?.trim().to_string();
        if let Some(v) = versions.iter().find(|v| v.commit == head) {
            return Err(format!(
                "the last saved version is already published as {}",
                v.name
            ));
        }
        let next = versions.iter().map(|v| v.number).max().unwrap_or(0) + 1;
        let name = format!("v{next}");
        let message = if message.trim().is_empty() {
            git(&dir, &["log", "-1", "--format=%s"])?.trim().to_string()
        } else {
            message.to_string()
        };
        git(&dir, &["tag", "-a", &name, "-m", &message])?;
        self.versions(id)?
            .into_iter()
            .find(|v| v.name == name)
            .ok_or_else(|| "the new version tag wasn't found".into())
    }

    /// Published versions, newest first.
    pub fn versions(&self, id: &str) -> Result<Vec<Version>, String> {
        let dir = self.dir(id)?;
        let out = git(
            &dir,
            &[
                "for-each-ref",
                "refs/tags/v*",
                "--format=%(refname:short)%1f%(creatordate:iso-strict)%1f%(contents:subject)%1f%(*objectname)%(objectname)",
            ],
        )?;
        let mut versions: Vec<Version> = out
            .lines()
            .filter_map(|line| {
                let f: Vec<&str> = line.split('\u{1f}').collect();
                let number = f.first()?.strip_prefix('v')?.parse().ok()?;
                // Annotated tags give the commit (%(*objectname)) then the tag id; keep the commit.
                let commit = f.get(3)?.get(..40)?.to_string();
                Some(Version {
                    name: f[0].to_string(),
                    number,
                    date: f.get(1)?.to_string(),
                    message: f.get(2)?.to_string(),
                    commit,
                })
            })
            .collect();
        versions.sort_by_key(|v| std::cmp::Reverse(v.number));
        Ok(versions)
    }

    pub fn latest_version(&self, id: &str) -> Result<Option<Version>, String> {
        Ok(self.versions(id)?.into_iter().next())
    }

    /// The tag for a version name: `v3`, or `latest` for the newest.
    pub fn version_tag(&self, id: &str, version: &str) -> Result<String, String> {
        let versions = self.versions(id)?;
        let found = if version == "latest" {
            versions.first()
        } else {
            versions.iter().find(|v| v.name == version)
        };
        found.map(|v| v.name.clone()).ok_or_else(|| match version {
            "latest" => format!("project `{id}` has no published version yet"),
            _ => format!("project `{id}` has no version `{version}`"),
        })
    }

    /// Saved versions, newest first.
    pub fn history(&self, id: &str) -> Result<Vec<Commit>, String> {
        let dir = self.dir(id)?;
        let versions = self.versions(id)?;
        let out = git(
            &dir,
            &["log", "--format=%H%x1f%h%x1f%aI%x1f%s", "-n", "200"],
        )?;
        Ok(out
            .lines()
            .filter_map(|line| {
                let f: Vec<&str> = line.split('\u{1f}').collect();
                let id = f.first()?.to_string();
                Some(Commit {
                    published: versions
                        .iter()
                        .find(|v| v.commit == id)
                        .map(|v| v.name.clone()),
                    short: f.get(1)?.to_string(),
                    date: f.get(2)?.to_string(),
                    message: f.get(3)?.to_string(),
                    id,
                })
            })
            .collect())
    }
}

/// Runs git in `dir` with a fixed identity and no user configuration
/// (hooks, signing) getting in the way.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=outliers-sim",
            "-c",
            "user.email=outliers-sim@localhost",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("can't run git (is it installed?): {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "git {}: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Example projects for an empty data folder: the examples shipped with the
/// program, each saved and published as v1.
pub fn seed_examples(projects: &Projects) -> Result<Vec<String>, String> {
    let examples = [
        (
            "conveyor-line",
            include_str!("../../../examples/projects/conveyor-line.yaml"),
            include_str!("../../../examples/conveyor-line.yaml"),
            include_str!("../../../examples/conveyor-line.layout.yaml"),
        ),
        (
            "two-cells",
            include_str!("../../../examples/projects/two-cells.yaml"),
            include_str!("../../../examples/two-cells.yaml"),
            include_str!("../../../examples/two-cells.layout.yaml"),
        ),
    ];
    let mut created = Vec::new();
    for (id, project, model, layout) in examples {
        let layout = layout
            .replace("model: conveyor-line.yaml", "model: model.yaml")
            .replace("model: two-cells.yaml", "model: model.yaml");
        projects.create(
            id,
            &[
                (PROJECT_FILE.into(), project.into()),
                ("model.yaml".into(), model.into()),
                ("model.layout.yaml".into(), layout),
            ],
            "Example project",
        )?;
        projects.publish(id, "First version")?;
        created.push(id.to_string());
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> Projects {
        let root = std::env::temp_dir().join(format!(
            "outliers-sim-projects-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Projects::new(root)
    }

    #[test]
    fn save_publish_and_read_versions() {
        let p = temp("versions");
        assert_eq!(seed_examples(&p).unwrap(), ["conveyor-line", "two-cells"]);
        assert_eq!(p.ids(), ["conveyor-line", "two-cells"]);
        let v1 = p.latest_version("two-cells").unwrap().unwrap();
        assert_eq!(
            (v1.name.as_str(), v1.message.as_str()),
            ("v1", "First version")
        );

        // Publishing again without saving is refused.
        assert!(
            p.publish("two-cells", "")
                .unwrap_err()
                .contains("already published as v1")
        );

        // Save a change: the working copy changes, v1 doesn't.
        let original = p.read("two-cells", None, "model.yaml").unwrap();
        let changed = original.replace(
            "interarrival: \"exponential(1 min)\"",
            "interarrival: \"exponential(2 min)\"",
        );
        assert_ne!(changed, original);
        let commit = p
            .save("two-cells", &[("model.yaml", &changed)], "Slower arrivals")
            .unwrap();
        assert_eq!(commit.message, "Slower arrivals");
        assert_eq!(commit.published, None);
        assert!(
            p.save("two-cells", &[("model.yaml", &changed)], "again")
                .unwrap_err()
                .contains("nothing changed")
        );
        assert_eq!(
            p.read("two-cells", Some("v1"), "model.yaml").unwrap(),
            original
        );
        assert_eq!(p.read("two-cells", None, "model.yaml").unwrap(), changed);

        let v2 = p.publish("two-cells", "").unwrap();
        // No message: the saved version's message describes it.
        assert_eq!(
            (v2.name.as_str(), v2.message.as_str()),
            ("v2", "Slower arrivals")
        );
        assert_eq!(
            p.read("two-cells", Some("latest"), "model.yaml").unwrap(),
            changed
        );
        let history = p.history("two-cells").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].published.as_deref(), Some("v2"));
        assert_eq!(history[1].published.as_deref(), Some("v1"));

        // Files that only exist in some versions.
        assert_eq!(
            p.read_optional("two-cells", Some("v1"), "nope.yaml")
                .unwrap(),
            None
        );
        assert!(
            p.read("two-cells", Some("v9"), "model.yaml")
                .unwrap_err()
                .contains("no version `v9`")
        );
        fs::remove_dir_all(&p.root).unwrap();
    }

    #[test]
    fn ids_are_checked() {
        let p = temp("ids");
        for bad in ["", "../x", "Two", "a/b", ".hidden"] {
            assert!(p.dir(bad).is_err(), "{bad}");
            assert!(p.create(bad, &[], "x").is_err(), "{bad}");
        }
        assert!(p.dir("missing").unwrap_err().contains("no project"));
        fs::remove_dir_all(&p.root).unwrap();
    }
}
