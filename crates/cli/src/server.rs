//! `outliers-sim serve`: the web application and its HTTP API.
//!
//! The web UI uses only this public API, so anything a user can do in the
//! browser, a program can do too. Simulations run on the server, one run at
//! a time on all cores; later runs wait in a queue. Models and scenarios are
//! YAML files in the models folder; each run's results go to its own folder.
//! See docs/api.md.

use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path as UrlPath, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use clap::Args;
use outliers_sim_model::{Model, ScenarioError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tower_http::compression::CompressionLayer;

use crate::files::{self, LoadFailure};
use crate::output::layout_json;
use crate::runner::{self, RunOptions};
use crate::thread_count;

#[derive(Args)]
pub struct ServeArgs {
    /// Folder with the model and scenario files (*.yaml).
    #[arg(long, default_value = "models")]
    models: PathBuf,
    /// Folder where each run's results are stored.
    #[arg(long, default_value = "runs")]
    runs: PathBuf,
    /// Address to listen on. The default only accepts connections from this
    /// machine; use 0.0.0.0 to serve other machines (behind IT's proxy).
    #[arg(long, default_value = "127.0.0.1")]
    host: IpAddr,
    #[arg(long, default_value_t = 8080)]
    port: u16,
    /// Threads per run [default: all CPU cores].
    #[arg(short, long)]
    threads: Option<usize>,
}

pub fn serve(args: &ServeArgs) -> Result<(), String> {
    if !args.models.is_dir() {
        return Err(format!(
            "the models folder {} doesn't exist",
            args.models.display()
        ));
    }
    fs::create_dir_all(&args.runs)
        .map_err(|e| format!("can't create {}: {e}", args.runs.display()))?;
    let state = AppState::start(
        args.models.clone(),
        args.runs.clone(),
        thread_count(args.threads),
    );
    let address = SocketAddr::new(args.host, args.port);
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(address)
            .await
            .map_err(|e| format!("can't listen on {address}: {e}"))?;
        println!(
            "serving {} on http://{address} (runs in {})",
            args.models.display(),
            args.runs.display()
        );
        axum::serve(listener, router(state))
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .map_err(|e| e.to_string())
    })
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/api/v1/models", get(list_models))
        .route("/api/v1/models/{file}", get(get_model).put(save_scenario))
        .route("/api/v1/check", axum::routing::post(check))
        .route("/api/v1/runs", get(list_runs).post(start_run))
        .route("/api/v1/runs/{id}", get(get_run))
        .route("/api/v1/runs/{id}/files/{name}", get(get_run_file))
        .layer(CompressionLayer::new())
        .with_state(state)
}

// --- state and the run queue ---------------------------------------------

pub struct AppState {
    models: PathBuf,
    runs: PathBuf,
    threads: usize,
    jobs: Mutex<Vec<Job>>,
    queue: mpsc::Sender<String>,
    next_id: AtomicU64,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Queued,
    Running,
    Done,
    Failed,
}

#[derive(Clone)]
struct Job {
    id: String,
    /// Model file name, or the scenario's name for unsaved scenarios.
    label: String,
    /// The model or scenario file to run (scenarios are written to the run folder).
    model_path: PathBuf,
    replications: Option<u32>,
    total: u32,
    progress: Arc<AtomicU32>,
    status: Status,
    error: Option<String>,
    files: Vec<String>,
}

impl Job {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "label": self.label,
            "status": self.status,
            "progress": { "done": self.progress.load(Ordering::Relaxed), "total": self.total },
            "error": self.error,
            "files": self.files,
        })
    }
}

impl AppState {
    /// Creates the state and starts the worker thread that runs queued jobs.
    pub fn start(models: PathBuf, runs: PathBuf, threads: usize) -> Arc<AppState> {
        let (queue, jobs_ready) = mpsc::channel::<String>();
        let state = Arc::new(AppState {
            models,
            runs,
            threads,
            jobs: Mutex::new(Vec::new()),
            queue,
            next_id: AtomicU64::new(1),
        });
        let worker = Arc::clone(&state);
        thread::spawn(move || {
            for id in jobs_ready {
                worker.run_job(&id);
            }
        });
        state
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut Job)) {
        let mut jobs = self.jobs.lock().expect("jobs lock");
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            f(job);
        }
    }

    fn run_job(&self, id: &str) {
        let Some(job) = self
            .jobs
            .lock()
            .expect("jobs lock")
            .iter()
            .find(|j| j.id == id)
            .cloned()
        else {
            return;
        };
        self.update(id, |j| j.status = Status::Running);
        let out_dir = self.runs.join(id);
        let result = (|| {
            let model = files::load_model(&job.model_path).map_err(|f| f.render())?;
            let layout =
                files::load_layout(&job.model_path, None, &model).map_err(|f| f.render())?;
            runner::execute(
                &model,
                &RunOptions {
                    replications: job.replications,
                    threads: self.threads,
                    log_replication: 1,
                    layout: layout.as_ref().map(|l| &l.layout),
                    progress: Some(&job.progress),
                },
                &out_dir,
            )
        })();
        self.update(id, |j| match result {
            Ok(outcome) => {
                j.status = Status::Done;
                j.files = outcome
                    .files
                    .iter()
                    .filter_map(|f| f.file_name()?.to_str().map(String::from))
                    .collect();
                if out_dir.join("scenario.yaml").exists() {
                    j.files.push("scenario.yaml".into());
                }
            }
            Err(error) => {
                j.status = Status::Failed;
                j.error = Some(error);
            }
        });
    }
}

// --- errors ------------------------------------------------------------------

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// Validation errors as JSON: the file each is in, and where.
fn problems(file_name: &str, diagnostics: &[outliers_sim_model::Diagnostic]) -> Vec<Value> {
    diagnostics
        .iter()
        .map(|d| {
            json!({
                "file": file_name,
                "line": d.position.map(|p| p.line),
                "column": d.position.map(|p| p.column),
                "message": d.message,
            })
        })
        .collect()
}

fn failure_problems(failure: &LoadFailure) -> Vec<Value> {
    let (file, diagnostics) = failure.diagnostics();
    let name = file
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or_default()
        .to_string();
    problems(&name, &diagnostics)
}

/// Model and scenario files are plain names in the models folder: no
/// paths, so requests can't reach outside it.
fn model_file(state: &AppState, name: &str) -> Result<PathBuf, String> {
    let ok = name.ends_with(".yaml")
        && !name.ends_with(".layout.yaml")
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(state.models.join(name))
    } else {
        Err(format!(
            "`{name}` is not a model file name (letters, digits, `.`, `_`, `-`, ending in .yaml)"
        ))
    }
}

// --- pages -----------------------------------------------------------------

/// The web interface: plain HTML and JS compiled into the binary.
const INDEX_HTML: &str = include_str!("web/index.html");
const APP_JS: &str = include_str!("web/app.js");

async fn index() -> Response {
    ([(header::CACHE_CONTROL, "no-cache")], Html(INDEX_HTML)).into_response()
}

async fn app_js() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        APP_JS,
    )
        .into_response()
}

// --- models ------------------------------------------------------------------

async fn list_models(State(state): State<Arc<AppState>>) -> Response {
    let Ok(entries) = fs::read_dir(&state.models) else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "can't read the models folder",
        );
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".yaml") && !n.ends_with(".layout.yaml") && !n.starts_with('.'))
        .collect();
    names.sort();
    let models: Vec<Value> = names
        .iter()
        .map(|name| {
            let path = state.models.join(name);
            let source = fs::read_to_string(&path).unwrap_or_default();
            let scenario = outliers_sim_model::is_scenario(&source);
            let loaded = files::load_model(&path);
            let base = scenario
                .then(|| outliers_sim_model::parse_scenario(&source).ok())
                .flatten()
                .map(|s| s.base.value);
            json!({
                "file": name,
                "kind": if scenario { "scenario" } else { "model" },
                "base": base,
                "name": loaded.as_ref().ok().map(|m| m.name.clone()),
                "description": loaded.as_ref().ok().and_then(|m| m.description.clone()),
                "valid": loaded.is_ok(),
            })
        })
        .collect();
    Json(json!({ "models": models })).into_response()
}

async fn get_model(State(state): State<Arc<AppState>>, UrlPath(name): UrlPath<String>) -> Response {
    let path = match model_file(&state, &name) {
        Ok(p) => p,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let Ok(source) = fs::read_to_string(&path) else {
        return error(StatusCode::NOT_FOUND, format!("no model `{name}`"));
    };
    let mut body = json!({ "file": name, "source": source });
    // The full model as a tree (for a scenario: with its changes applied):
    // what scenario change paths address, and what forms display.
    if outliers_sim_model::is_scenario(&source) {
        body["kind"] = json!("scenario");
        if let Ok(s) = outliers_sim_model::parse_scenario(&source) {
            body["base"] = json!(s.base.value);
            body["changes"] = json!(
                s.changes()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect::<serde_json::Map<_, _>>()
            );
            let base_source =
                fs::read_to_string(files::scenario_base(&path, &s.base.value)).unwrap_or_default();
            if let Ok(merged) = outliers_sim_model::merge(&s, &base_source) {
                body["tree"] = outliers_sim_model::to_tree(&merged.text).unwrap_or(Value::Null);
            }
        }
    } else {
        body["kind"] = json!("model");
        body["tree"] = outliers_sim_model::to_tree(&source).unwrap_or(Value::Null);
    }
    match files::load_model(&path) {
        Ok(model) => {
            body["valid"] = json!(true);
            body["errors"] = json!([]);
            body["layout"] = match files::load_layout(&path, None, &model) {
                Ok(Some(l)) => layout_json(&l.layout),
                _ => Value::Null,
            };
        }
        Err(failure) => {
            body["valid"] = json!(false);
            body["errors"] = json!(failure_problems(&failure));
            body["layout"] = Value::Null;
        }
    }
    Json(body).into_response()
}

/// An unsaved scenario, as the UI and API clients send it.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioBody {
    name: Option<String>,
    description: Option<String>,
    /// The base model's file name in the models folder.
    base: String,
    #[serde(default)]
    changes: serde_json::Map<String, Value>,
}

impl ScenarioBody {
    /// The scenario as a YAML file, with `base` as given. Returns the text
    /// and the line number of the first change (one change per line).
    fn to_yaml(&self, base: &str) -> Result<(String, u64), String> {
        if let Some(key) = self.changes.keys().find(|k| {
            k.is_empty()
                || !k
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        }) {
            return Err(format!(
                "`{key}` is not a change path (names separated by dots)"
            ));
        }
        let name = self.name.as_deref().unwrap_or("unsaved-scenario");
        let mut text = format!(
            "format: {}\nname: {}\n",
            outliers_sim_model::SCENARIO_FORMAT,
            json!(name)
        );
        if let Some(d) = &self.description {
            text += &format!("description: {}\n", json!(d));
        }
        text += &format!("base: {}\n", json!(base));
        let first_change_line = text.lines().count() as u64 + 2;
        if self.changes.is_empty() {
            text += "changes: {}\n";
        } else {
            text += "changes:\n";
            for (path, value) in &self.changes {
                text += &format!("  {path}: {value}\n");
            }
        }
        Ok((text, first_change_line))
    }

    /// Validates the scenario against its base. Errors name the change they
    /// come from (or the base file), so the UI can mark the right field.
    fn check(&self, state: &AppState) -> Result<Model, Vec<Value>> {
        let one =
            |message: String| vec![json!({ "change": null, "file": null, "message": message })];
        let base_path = model_file(state, &self.base)
            .map_err(|_| one(format!("`{}` is not a model file name", self.base)))?;
        let base_source = fs::read_to_string(&base_path)
            .map_err(|_| one(format!("no base model `{}`", self.base)))?;
        if outliers_sim_model::is_scenario(&base_source) {
            return Err(one(format!(
                "`{}` is a scenario; a scenario's base must be a model",
                self.base
            )));
        }
        let (text, first_change_line) = self.to_yaml(&self.base).map_err(one)?;
        let keys: Vec<&String> = self.changes.keys().collect();
        let in_scenario = |diagnostics: &[outliers_sim_model::Diagnostic]| -> Vec<Value> {
            diagnostics
                .iter()
                .map(|d| {
                    let change = d
                        .position
                        .and_then(|p| p.line.checked_sub(first_change_line))
                        .and_then(|i| keys.get(i as usize));
                    // The change is its own field; drop the CLI's "with this change:" prefix.
                    let message = d
                        .message
                        .strip_prefix("with this change: ")
                        .unwrap_or(&d.message);
                    json!({ "change": change, "file": null, "message": message })
                })
                .collect()
        };
        let scenario =
            outliers_sim_model::parse_scenario(&text).map_err(|e| in_scenario(e.diagnostics()))?;
        match outliers_sim_model::load_scenario(&scenario, &base_source) {
            Ok(model) => Ok(model),
            Err(ScenarioError::Scenario(e)) => Err(in_scenario(e.diagnostics())),
            Err(ScenarioError::Base(e)) => Err(e
                .diagnostics()
                .iter()
                .map(|d| json!({ "change": null, "file": self.base, "line": d.position.map(|p| p.line), "message": d.message }))
                .collect()),
        }
    }
}

async fn check(State(state): State<Arc<AppState>>, Json(body): Json<ScenarioBody>) -> Response {
    match body.check(&state) {
        Ok(_) => Json(json!({ "ok": true, "errors": [] })).into_response(),
        Err(errors) => Json(json!({ "ok": false, "errors": errors })).into_response(),
    }
}

/// Saves a scenario file. Base models are never overwritten through the API.
async fn save_scenario(
    State(state): State<Arc<AppState>>,
    UrlPath(name): UrlPath<String>,
    Json(body): Json<ScenarioBody>,
) -> Response {
    let path = match model_file(&state, &name) {
        Ok(p) => p,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    if let Ok(existing) = fs::read_to_string(&path)
        && !outliers_sim_model::is_scenario(&existing)
    {
        return error(
            StatusCode::CONFLICT,
            format!("`{name}` is a base model; only scenarios can be saved through the API"),
        );
    }
    if let Err(errors) = body.check(&state) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "ok": false, "errors": errors })),
        )
            .into_response();
    }
    let text = match body.to_yaml(&body.base) {
        Ok((text, _)) => text,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    match fs::write(&path, text) {
        Ok(()) => Json(json!({ "ok": true, "file": name })).into_response(),
        Err(e) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("can't write `{name}`: {e}"),
        ),
    }
}

// --- runs --------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunRequest {
    /// A model or scenario file in the models folder...
    file: Option<String>,
    /// ...or an unsaved scenario.
    scenario: Option<ScenarioBody>,
    /// Overrides the model's number of replications.
    replications: Option<u32>,
}

async fn start_run(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RunRequest>,
) -> Response {
    if request.replications == Some(0) {
        return error(StatusCode::BAD_REQUEST, "`replications` must be at least 1");
    }
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let id = format!("{secs}-{}", state.next_id.fetch_add(1, Ordering::Relaxed));
    let run_dir = state.runs.join(&id);

    let (label, model_path, model) = match (&request.file, &request.scenario) {
        (Some(file), None) => {
            let path = match model_file(&state, file) {
                Ok(p) => p,
                Err(e) => return error(StatusCode::BAD_REQUEST, e),
            };
            match files::load_model(&path) {
                Ok(model) => (file.clone(), path, model),
                Err(failure) => {
                    return (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        Json(json!({ "error": "the model has errors", "errors": failure_problems(&failure) })),
                    )
                        .into_response();
                }
            }
        }
        (None, Some(scenario)) => {
            let model = match scenario.check(&state) {
                Ok(model) => model,
                Err(errors) => {
                    return (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        Json(json!({ "error": "the scenario has errors", "errors": errors })),
                    )
                        .into_response();
                }
            };
            // Keep the scenario with its results, pointing at the base by
            // absolute path so the run folder can be anywhere.
            let base = fs::canonicalize(state.models.join(&scenario.base)).unwrap_or_default();
            let (text, _) = scenario.to_yaml(&base.to_string_lossy()).expect("checked");
            let path = run_dir.join("scenario.yaml");
            if let Err(e) = fs::create_dir_all(&run_dir).and_then(|()| fs::write(&path, text)) {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("can't write the run folder: {e}"),
                );
            }
            (model.name.clone(), path, model)
        }
        _ => return error(StatusCode::BAD_REQUEST, "give either `file` or `scenario`"),
    };

    let job = Job {
        id: id.clone(),
        label,
        model_path,
        replications: request.replications,
        total: request
            .replications
            .unwrap_or(model.spec.experiment.replications),
        progress: Arc::new(AtomicU32::new(0)),
        status: Status::Queued,
        error: None,
        files: Vec::new(),
    };
    let body = job.to_json();
    state.jobs.lock().expect("jobs lock").push(job);
    if state.queue.send(id).is_err() {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the run queue has stopped",
        );
    }
    (StatusCode::ACCEPTED, Json(body)).into_response()
}

async fn list_runs(State(state): State<Arc<AppState>>) -> Response {
    let jobs = state.jobs.lock().expect("jobs lock");
    let runs: Vec<Value> = jobs.iter().rev().map(Job::to_json).collect();
    Json(json!({ "runs": runs })).into_response()
}

async fn get_run(State(state): State<Arc<AppState>>, UrlPath(id): UrlPath<String>) -> Response {
    let jobs = state.jobs.lock().expect("jobs lock");
    match jobs.iter().find(|j| j.id == id) {
        Some(job) => Json(job.to_json()).into_response(),
        None => error(StatusCode::NOT_FOUND, format!("no run `{id}`")),
    }
}

/// Result files a run can have, and their media types.
const RUN_FILES: &[(&str, &str)] = &[
    ("report.html", "text/html; charset=utf-8"),
    ("summary.json", "application/json"),
    ("replications.csv", "text/csv; charset=utf-8"),
    ("events.jsonl", "application/x-ndjson"),
    ("layout.json", "application/json"),
    ("scenario.yaml", "text/yaml; charset=utf-8"),
];

async fn get_run_file(
    State(state): State<Arc<AppState>>,
    UrlPath((id, name)): UrlPath<(String, String)>,
) -> Response {
    let known = state
        .jobs
        .lock()
        .expect("jobs lock")
        .iter()
        .any(|j| j.id == id);
    let Some((_, media_type)) = RUN_FILES.iter().find(|(n, _)| *n == name) else {
        return error(StatusCode::NOT_FOUND, format!("runs have no file `{name}`"));
    };
    if !known {
        return error(StatusCode::NOT_FOUND, format!("no run `{id}`"));
    }
    match fs::read(state.runs.join(&id).join(&name)) {
        Ok(bytes) => ([(header::CONTENT_TYPE, *media_type)], bytes).into_response(),
        Err(_) => error(
            StatusCode::NOT_FOUND,
            format!("run `{id}` has no `{name}` (yet)"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// A server over a copy of the examples, with its own runs folder.
    fn app(name: &str) -> (Router, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("outliers-sim-serve-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let models = root.join("models");
        fs::create_dir_all(&models).unwrap();
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        for entry in fs::read_dir(examples).unwrap() {
            let path = entry.unwrap().path();
            fs::copy(&path, models.join(path.file_name().unwrap())).unwrap();
        }
        let state = AppState::start(models, root.join("runs"), 2);
        (router(state), root)
    }

    async fn call(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into()));
        (status, value)
    }

    async fn wait_for(app: &Router, id: &str) -> Value {
        for _ in 0..600 {
            let (_, run) = call(app, "GET", &format!("/api/v1/runs/{id}"), None).await;
            if run["status"] == "done" || run["status"] == "failed" {
                return run;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("run {id} didn't finish");
    }

    #[tokio::test]
    async fn lists_models_and_scenarios() {
        let (app, root) = app("list");
        let (status, body) = call(&app, "GET", "/api/v1/models", None).await;
        assert_eq!(status, StatusCode::OK);
        let models = body["models"].as_array().unwrap();
        let find = |f: &str| models.iter().find(|m| m["file"] == f).unwrap().clone();
        assert_eq!(find("two-cells.yaml")["kind"], "model");
        let scenario = find("two-cells-round-robin.yaml");
        assert_eq!(scenario["kind"], "scenario");
        assert_eq!(scenario["base"], "two-cells.yaml");
        assert_eq!(scenario["valid"], true);
        assert!(
            models
                .iter()
                .all(|m| !m["file"].as_str().unwrap().ends_with(".layout.yaml"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn model_detail_has_tree_and_layout() {
        let (app, root) = app("detail");
        let (_, body) = call(
            &app,
            "GET",
            "/api/v1/models/two-cells-first-available.yaml",
            None,
        )
        .await;
        assert_eq!(body["kind"], "scenario");
        assert_eq!(body["valid"], true);
        // The tree has the change applied, at the path the change names.
        assert_eq!(
            body["tree"]["sources"]["arrivals"]["output"]["rule"],
            "first_available"
        );
        assert_eq!(
            body["changes"]["sources.arrivals.output"]["rule"],
            "first_available"
        );
        assert_eq!(body["layout"]["format"], "outliers-sim-layout/0.1");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn check_names_the_change_at_fault() {
        let (app, root) = app("check");
        let scenario = json!({
            "base": "two-cells.yaml",
            "changes": {
                "conveyors.c_a.length": "12 m",
                "sources.arrivals.output": { "rule": "round_robn" },
                "servers.mil_b.capacity": 2,
            }
        });
        let (_, body) = call(&app, "POST", "/api/v1/check", Some(scenario)).await;
        assert_eq!(body["ok"], false);
        let errors = body["errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1, "{errors:?}"); // bad paths stop before validation
        assert_eq!(errors[0]["change"], "servers.mil_b.capacity");
        assert!(
            errors[0]["message"]
                .as_str()
                .unwrap()
                .contains("has no `mil_b`")
        );

        let scenario = json!({
            "base": "two-cells.yaml",
            "changes": { "conveyors.c_a.length": "12 m", "sources.arrivals.output": { "rule": "round_robn" } }
        });
        let (_, body) = call(&app, "POST", "/api/v1/check", Some(scenario)).await;
        assert_eq!(body["errors"][0]["change"], "sources.arrivals.output");
        assert!(
            body["errors"][0]["message"]
                .as_str()
                .unwrap()
                .contains("round_robn")
        );

        let ok = json!({ "base": "two-cells.yaml", "changes": { "conveyors.c_a.length": "12 m" } });
        let (_, body) = call(&app, "POST", "/api/v1/check", Some(ok)).await;
        assert_eq!(body, json!({ "ok": true, "errors": [] }));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn saves_scenarios_but_never_base_models() {
        let (app, root) = app("save");
        let scenario = json!({
            "name": "bigger-buffer",
            "base": "conveyor-line.yaml",
            "changes": { "conveyors.c2.length": "16 m" }
        });
        let (status, _) = call(
            &app,
            "PUT",
            "/api/v1/models/bigger-buffer.yaml",
            Some(scenario.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let saved = fs::read_to_string(root.join("models/bigger-buffer.yaml")).unwrap();
        assert!(saved.contains("  conveyors.c2.length: \"16 m\""), "{saved}");
        let (_, detail) = call(&app, "GET", "/api/v1/models/bigger-buffer.yaml", None).await;
        assert_eq!(detail["valid"], true);

        let (status, body) = call(
            &app,
            "PUT",
            "/api/v1/models/conveyor-line.yaml",
            Some(scenario),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");

        let bad =
            json!({ "base": "conveyor-line.yaml", "changes": { "conveyors.c2.length": "16 min" } });
        let (status, body) = call(&app, "PUT", "/api/v1/models/bad.yaml", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["errors"][0]["change"], "conveyors.c2.length");
        assert!(!root.join("models/bad.yaml").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn runs_a_saved_model_and_an_unsaved_scenario() {
        let (app, root) = app("runs");
        let (status, run) = call(
            &app,
            "POST",
            "/api/v1/runs",
            Some(json!({ "file": "two-cells.yaml", "replications": 3 })),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let run = wait_for(&app, run["id"].as_str().unwrap()).await;
        assert_eq!(run["status"], "done", "{run}");
        assert_eq!(run["progress"], json!({ "done": 3, "total": 3 }));
        for file in [
            "report.html",
            "summary.json",
            "replications.csv",
            "events.jsonl",
            "layout.json",
        ] {
            assert!(
                run["files"].as_array().unwrap().contains(&json!(file)),
                "{run}"
            );
        }
        let id = run["id"].as_str().unwrap();
        let (status, summary) = call(
            &app,
            "GET",
            &format!("/api/v1/runs/{id}/files/summary.json"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(summary["replications"], 3);

        let scenario = json!({
            "scenario": { "base": "two-cells.yaml", "changes": { "sources.arrivals.output": { "rule": "round_robin" } } },
            "replications": 2
        });
        let (status, run) = call(&app, "POST", "/api/v1/runs", Some(scenario)).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{run}");
        let run = wait_for(&app, run["id"].as_str().unwrap()).await;
        assert_eq!(run["status"], "done", "{run}");
        assert!(
            run["files"]
                .as_array()
                .unwrap()
                .contains(&json!("layout.json")),
            "{run}"
        );
        let id = run["id"].as_str().unwrap();
        assert!(root.join("runs").join(id).join("scenario.yaml").exists());

        let (status, body) = call(&app, "POST", "/api/v1/runs", Some(json!({ "scenario": { "base": "two-cells.yaml", "changes": { "experiment.replications": 0 } } }))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["errors"][0]["change"], "experiment.replications");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn serves_the_web_interface() {
        let (app, root) = app("web");
        for (uri, media_type, marker) in [
            ("/", "text/html", "<script src=\"app.js\">"),
            ("/app.js", "text/javascript", "const API = \"api/v1\";"),
        ] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            let content_type = response.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .to_string();
            assert!(
                content_type.starts_with(media_type),
                "{uri}: {content_type}"
            );
            let body = response.into_body().collect().await.unwrap().to_bytes();
            assert!(String::from_utf8_lossy(&body).contains(marker), "{uri}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn rejects_paths_outside_the_models_folder() {
        let (app, root) = app("paths");
        for uri in [
            "/api/v1/models/..%2Fsecret.yaml",
            "/api/v1/models/.hidden.yaml",
            "/api/v1/models/x.layout.yaml",
        ] {
            let (status, _) = call(&app, "GET", uri, None).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        }
        let (status, _) = call(
            &app,
            "GET",
            "/api/v1/runs/nope/files/../../etc/passwd",
            None,
        )
        .await;
        assert_ne!(status, StatusCode::OK);
        let (status, _) = call(&app, "GET", "/api/v1/runs/nope/files/report.html", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        fs::remove_dir_all(root).unwrap();
    }
}
