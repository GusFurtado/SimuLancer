//! Loading models, scenarios and layouts from files, with errors kept as
//! data so the command line can print them and the server can return them.

use std::fs;
use std::path::{Path, PathBuf};

use simulancer_model::{Layout, LoadError, Model, ScenarioError};

/// Why a file couldn't be loaded.
#[derive(Debug)]
pub enum LoadFailure {
    Io { file: PathBuf, error: String },
    Invalid(Box<InvalidFile>),
}

/// A file that was read but has errors.
#[derive(Debug)]
pub struct InvalidFile {
    pub file: PathBuf,
    pub source: String,
    pub error: LoadError,
    /// Set when `file` is the base model of this scenario.
    pub base_of: Option<PathBuf>,
}

fn invalid(
    file: PathBuf,
    source: String,
    error: LoadError,
    base_of: Option<PathBuf>,
) -> LoadFailure {
    LoadFailure::Invalid(Box::new(InvalidFile {
        file,
        source,
        error,
        base_of,
    }))
}

impl LoadFailure {
    /// The errors as the command line prints them, with source snippets.
    pub fn render(&self) -> String {
        match self {
            LoadFailure::Io { file, error } => {
                format!("error: can't read {}: {error}\n", file.display())
            }
            LoadFailure::Invalid(f) => {
                let mut text = f.error.render(&f.file.display().to_string(), &f.source);
                if let Some(scenario) = &f.base_of {
                    text += &format!(
                        "(in {}, the base model of {})\n",
                        f.file.display(),
                        scenario.display()
                    );
                }
                text
            }
        }
    }
}

pub fn read(path: &Path) -> Result<String, LoadFailure> {
    fs::read_to_string(path).map_err(|e| LoadFailure::Io {
        file: path.to_path_buf(),
        error: e.to_string(),
    })
}

/// Loads a model or a scenario file.
pub fn load_model(path: &Path) -> Result<Model, LoadFailure> {
    let source = read(path)?;
    if !simulancer_model::is_scenario(&source) {
        return simulancer_model::load_str(&source)
            .map_err(|error| invalid(path.to_path_buf(), source, error, None));
    }
    let scenario = match simulancer_model::parse_scenario(&source) {
        Ok(s) => s,
        Err(error) => return Err(invalid(path.to_path_buf(), source, error, None)),
    };
    let base_path = scenario_base(path, &scenario.base.value);
    let base_source = read(&base_path)?;
    match simulancer_model::load_scenario(&scenario, &base_source) {
        Ok(model) => Ok(model),
        Err(ScenarioError::Scenario(error)) => {
            Err(invalid(path.to_path_buf(), source, error, None))
        }
        Err(ScenarioError::Base(error)) => Err(invalid(
            base_path,
            base_source,
            error,
            Some(path.to_path_buf()),
        )),
    }
}

/// The base model of a scenario: its `base:` path, relative to the scenario file.
pub fn scenario_base(scenario_path: &Path, base: &str) -> PathBuf {
    scenario_path.parent().unwrap_or(Path::new("")).join(base)
}

/// The model whose layout a file uses: the file itself, or a scenario's base.
pub fn layout_owner(path: &Path) -> PathBuf {
    fs::read_to_string(path)
        .ok()
        .filter(|source| simulancer_model::is_scenario(source))
        .and_then(|source| simulancer_model::parse_scenario(&source).ok())
        .map_or_else(
            || path.to_path_buf(),
            |s| scenario_base(path, &s.base.value),
        )
}

/// The layout to use: the one given, or `<model>.layout.yaml` next to the model.
pub fn find_layout(model: &Path, explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }
    let stem = model.file_stem()?.to_str()?;
    let sibling = model.with_file_name(format!("{stem}.layout.yaml"));
    sibling.exists().then_some(sibling)
}

/// A checked layout and its warnings, rendered like errors.
pub struct LoadedLayout {
    pub layout: Layout,
    pub warnings: Vec<String>,
}

/// Loads the layout for a model file, if it has one.
pub fn load_layout(
    model_path: &Path,
    explicit: Option<&Path>,
    model: &Model,
) -> Result<Option<LoadedLayout>, LoadFailure> {
    let Some(path) = find_layout(&layout_owner(model_path), explicit) else {
        return Ok(None);
    };
    let source = read(&path)?;
    match simulancer_model::load_layout(&source, model) {
        Ok((layout, warnings)) => Ok(Some(LoadedLayout {
            layout,
            warnings: warnings
                .iter()
                .map(|w| w.render_as("warning", &path.display().to_string(), &source))
                .collect(),
        })),
        Err(error) => Err(invalid(path, source, error, None)),
    }
}
