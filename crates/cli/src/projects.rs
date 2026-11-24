//! Projects: one folder per project, each a local git repository.
//!
//! ```text
//! projects/<id>/
//!   project.yaml        name, description, which file is the model
//!   model.yaml          the model
//!   model.layout.yaml   its layout (optional)
//!   README.md           its documentation, in Markdown (optional)
//! projects/.archive/<id>/   archived projects, the same way
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

pub const PROJECT_FORMAT: &str = "simulancer-project/0.1";
pub const PROJECT_FILE: &str = "project.yaml";
pub const DOCUMENTATION_FILE: &str = "README.md";

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
        simulancer_model::to_yaml(&tree)
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

    /// Where archived projects go. Its name isn't a project id, so it's never listed as one.
    fn archive_root(&self) -> PathBuf {
        self.root.join(".archive")
    }

    fn archived_dir(&self, id: &str) -> Result<PathBuf, String> {
        let dir = self.archive_root().join(id);
        if valid_id(id) && dir.join(PROJECT_FILE).is_file() {
            Ok(dir)
        } else {
            Err(format!("no archived project `{id}`"))
        }
    }

    /// Archived project ids, sorted.
    pub fn archived_ids(&self) -> Vec<String> {
        let root = self.archive_root();
        let mut ids: Vec<String> = fs::read_dir(&root)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|id| valid_id(id) && root.join(id).join(PROJECT_FILE).is_file())
            .collect();
        ids.sort();
        ids
    }

    /// An archived project's `project.yaml`.
    pub fn archived_project_file(&self, id: &str) -> Result<ProjectFile, String> {
        let dir = self.archived_dir(id)?;
        let text = fs::read_to_string(dir.join(PROJECT_FILE)).map_err(|e| e.to_string())?;
        parse_project_file(id, &text)
    }

    /// Hides a project from the list, keeping all its files and history.
    pub fn archive(&self, id: &str) -> Result<(), String> {
        let dir = self.dir(id)?;
        let target = self.archive_root().join(id);
        if target.exists() {
            return Err(format!("an archived project `{id}` already exists"));
        }
        fs::create_dir_all(self.archive_root()).map_err(|e| e.to_string())?;
        fs::rename(dir, target).map_err(|e| e.to_string())
    }

    /// Brings an archived project back to the list.
    pub fn restore(&self, id: &str) -> Result<(), String> {
        let dir = self.archived_dir(id)?;
        let target = self.root.join(id);
        if target.exists() {
            return Err(format!("a project `{id}` already exists"));
        }
        fs::rename(dir, target).map_err(|e| e.to_string())
    }

    /// Deletes an archived project for good. Only archived projects can be
    /// deleted, so deleting always takes two steps.
    pub fn delete_archived(&self, id: &str) -> Result<(), String> {
        let dir = self.archived_dir(id)?;
        fs::remove_dir_all(dir).map_err(|e| e.to_string())
    }

    /// Changes the project's name and description, saved as a new version.
    /// Returns false when they were already like that.
    pub fn set_details(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
    ) -> Result<bool, String> {
        let mut file = self.project_file(id, None)?;
        let mut what = Vec::new();
        if let Some(name) = name.map(str::trim) {
            if name.is_empty() {
                return Err("a project needs a name".into());
            }
            if name != file.name {
                file.name = name.to_string();
                what.push(format!("Renamed to {name}"));
            }
        }
        if let Some(description) = description.map(str::trim) {
            let description = (!description.is_empty()).then(|| description.to_string());
            if description != file.description {
                file.description = description;
                what.push("Changed the description".to_string());
            }
        }
        if what.is_empty() {
            return Ok(false);
        }
        self.save(
            id,
            &[(PROJECT_FILE, Some(&file.to_text()))],
            &what.join("; "),
        )?;
        Ok(true)
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
        // Its scenarios are kept under the same id, so the id stays taken.
        if self.archive_root().join(id).exists() {
            return Err(format!(
                "a project `{id}` already exists in the archive; restore or delete it first"
            ));
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
        parse_project_file(id, &text)
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

    /// Writes files and saves them as a new version (a commit). A file
    /// with no content (`None`) is removed.
    pub fn save(
        &self,
        id: &str,
        files: &[(&str, Option<&str>)],
        message: &str,
    ) -> Result<Commit, String> {
        let dir = self.dir(id)?;
        for (name, content) in files {
            let result = match content {
                Some(content) => fs::write(dir.join(name), content),
                None => match fs::remove_file(dir.join(name)) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    other => other,
                },
            };
            result.map_err(|e| e.to_string())?;
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

fn parse_project_file(id: &str, text: &str) -> Result<ProjectFile, String> {
    let file: ProjectFile =
        serde_saphyr::from_str(text).map_err(|e| format!("{id}/{PROJECT_FILE}: {e}"))?;
    if file.format != PROJECT_FORMAT {
        return Err(format!(
            "{id}/{PROJECT_FILE}: unsupported format `{}` (this version reads `{PROJECT_FORMAT}`)",
            file.format
        ));
    }
    Ok(file)
}

/// Runs git in `dir` with a fixed identity and no user configuration
/// (hooks, signing) getting in the way.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=simulancer",
            "-c",
            "user.email=simulancer@localhost",
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

/// A blank project's files: a simple line (source, server, sink) to build on.
/// (An empty model isn't valid: a model needs a source and a sink.)
pub fn blank(id: &str, name: &str) -> Vec<(String, String)> {
    let project = ProjectFile {
        format: PROJECT_FORMAT.into(),
        name: name.into(),
        description: None,
        model: "model.yaml".into(),
        decisions: Map::new(),
        results: Vec::new(),
    };
    vec![
        (PROJECT_FILE.into(), project.to_text()),
        (
            "model.yaml".into(),
            include_str!("../../../examples/blank.yaml")
                .replace("name: new-line", &format!("name: {id}")),
        ),
        (
            "model.layout.yaml".into(),
            include_str!("../../../examples/blank.layout.yaml")
                .replace("model: blank.yaml", "model: model.yaml"),
        ),
    ]
}

/// A model shipped with the program that new projects can start from. The
/// template itself never changes: a project gets a copy.
pub struct Template {
    pub id: &'static str,
    pub project: &'static str,
    pub model: &'static str,
    pub layout: &'static str,
    /// Its documentation: what it assumes and leaves out.
    pub readme: &'static str,
}

macro_rules! template {
    ($id:literal) => {
        Template {
            id: $id,
            project: include_str!(concat!("../../../examples/projects/", $id, ".yaml")),
            model: include_str!(concat!("../../../examples/", $id, ".yaml")),
            layout: include_str!(concat!("../../../examples/", $id, ".layout.yaml")),
            readme: include_str!(concat!("../../../examples/projects/", $id, ".md")),
        }
    };
}

/// The templates, in the gallery's order. Their tags are in `examples/templates.yaml`.
pub const TEMPLATES: &[Template] = &[
    template!("assembly-line"),
    template!("bank-branch"),
    template!("warehouse"),
    template!("bike-assembly"),
    template!("conveyor-line"),
    template!("two-cells"),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    templates: std::collections::BTreeMap<String, CatalogEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogEntry {
    tags: Vec<String>,
}

static CATALOG: std::sync::LazyLock<Catalog> = std::sync::LazyLock::new(|| {
    serde_saphyr::from_str(include_str!("../../../examples/templates.yaml"))
        .expect("examples/templates.yaml is valid")
});

impl Template {
    /// Its `project.yaml`, as written.
    pub fn project_file(&self) -> ProjectFile {
        parse_project_file(self.id, self.project).expect("the shipped templates are valid")
    }

    /// What it's about and what it shows, for searching the gallery.
    pub fn tags(&self) -> &'static [String] {
        CATALOG.templates.get(self.id).map_or(&[], |e| &e.tags)
    }
}

/// Creates a project from a template, saved and published as v1 so its
/// Simulation page works right away. `name` replaces the template's.
pub fn create_from_template(
    projects: &Projects,
    id: &str,
    template: &str,
    name: Option<&str>,
) -> Result<(), String> {
    let template = TEMPLATES
        .iter()
        .find(|t| t.id == template)
        .ok_or_else(|| format!("no template `{template}`"))?;
    let mut file = template.project_file();
    let message = format!("Started from the {} template", file.name);
    let project = match name {
        Some(name) => {
            file.name = name.to_string();
            file.to_text()
        }
        None => template.project.to_string(),
    };
    let layout = template
        .layout
        .replace(&format!("model: {}.yaml", template.id), "model: model.yaml");
    projects.create(
        id,
        &[
            (PROJECT_FILE.into(), project),
            ("model.yaml".into(), template.model.into()),
            ("model.layout.yaml".into(), layout),
            (DOCUMENTATION_FILE.into(), template.readme.into()),
        ],
        &message,
    )?;
    projects.publish(id, "First version")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> Projects {
        let root =
            std::env::temp_dir().join(format!("simulancer-projects-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Projects::new(root)
    }

    #[test]
    fn every_template_is_in_the_catalog_with_tags() {
        let ids: Vec<&str> = TEMPLATES.iter().map(|t| t.id).collect();
        let catalog: Vec<&str> = CATALOG.templates.keys().map(String::as_str).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(
            sorted, catalog,
            "examples/templates.yaml and TEMPLATES list the same ids"
        );
        for t in TEMPLATES {
            assert!(!t.tags().is_empty(), "{} has tags", t.id);
            t.project_file();
        }
    }

    #[test]
    fn rename_archive_restore_and_delete() {
        let p = temp("archive");
        for id in ["bike-assembly", "conveyor-line", "two-cells"] {
            create_from_template(&p, id, id, None).unwrap();
        }

        // Name and description are saved as a version; nothing changed, nothing saved.
        assert!(
            p.set_details("two-cells", Some(" Two mills "), Some("New text"))
                .unwrap()
        );
        let file = p.project_file("two-cells", None).unwrap();
        assert_eq!(
            (file.name.as_str(), file.description.as_deref()),
            ("Two mills", Some("New text"))
        );
        assert_eq!(
            p.history("two-cells").unwrap()[0].message,
            "Renamed to Two mills; Changed the description"
        );
        assert!(!p.set_details("two-cells", Some("Two mills"), None).unwrap());
        assert!(p.set_details("two-cells", Some("  "), None).is_err());
        assert!(p.set_details("two-cells", None, Some("")).unwrap());
        assert_eq!(p.project_file("two-cells", None).unwrap().description, None);

        // Archived: off the list, but kept, and its id stays taken.
        p.archive("two-cells").unwrap();
        assert_eq!(p.ids(), ["bike-assembly", "conveyor-line"]);
        assert_eq!(p.archived_ids(), ["two-cells"]);
        assert_eq!(
            p.archived_project_file("two-cells").unwrap().name,
            "Two mills"
        );
        assert!(p.dir("two-cells").is_err());
        assert!(
            p.create("two-cells", &blank("two-cells", "Again"), "x")
                .unwrap_err()
                .contains("in the archive")
        );
        // Only archived projects can be deleted.
        assert!(p.delete_archived("conveyor-line").is_err());

        p.restore("two-cells").unwrap();
        assert_eq!(p.ids(), ["bike-assembly", "conveyor-line", "two-cells"]);
        assert_eq!(p.history("two-cells").unwrap().len(), 3);
        assert!(p.restore("two-cells").is_err());

        p.archive("conveyor-line").unwrap();
        p.delete_archived("conveyor-line").unwrap();
        assert!(p.archived_ids().is_empty());
        assert_eq!(p.ids(), ["bike-assembly", "two-cells"]);
        fs::remove_dir_all(&p.root).unwrap();
    }

    #[test]
    fn save_publish_and_read_versions() {
        let p = temp("versions");
        create_from_template(&p, "two-cells", "two-cells", None).unwrap();
        assert_eq!(p.ids(), ["two-cells"]);
        assert_eq!(
            p.history("two-cells").unwrap()[0].message,
            "Started from the Two cells template"
        );
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
            .save(
                "two-cells",
                &[("model.yaml", Some(&changed))],
                "Slower arrivals",
            )
            .unwrap();
        assert_eq!(commit.message, "Slower arrivals");
        assert_eq!(commit.published, None);
        assert!(
            p.save("two-cells", &[("model.yaml", Some(&changed))], "again")
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
    fn project_files_round_trip() {
        for text in [
            include_str!("../../../examples/projects/two-cells.yaml"),
            include_str!("../../../examples/projects/conveyor-line.yaml"),
            include_str!("../../../examples/projects/bike-assembly.yaml"),
        ] {
            let file: ProjectFile = serde_saphyr::from_str(text).unwrap();
            let again: ProjectFile = serde_saphyr::from_str(&file.to_text()).unwrap();
            assert_eq!(
                serde_json::to_value(&file).unwrap(),
                serde_json::to_value(&again).unwrap(),
                "{}",
                file.to_text()
            );
        }
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
