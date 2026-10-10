//! `outliers-sim serve`: the web application and its HTTP API.
//!
//! Three audiences, three interfaces over the same API:
//! - engineers build and version models (`/build`),
//! - managers compare scenarios on published versions (`/decide`),
//! - developers use the HTTP API directly (docs/api.md).
//!
//! The web pages use only the public API. Simulations run on the server, one
//! run at a time on all cores; later runs wait in a queue. Projects are git
//! repositories in `<data>/projects`; each run's results go to `<data>/runs/<id>`.

use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Args;
use outliers_sim_model::{Diagnostic, Layout, Model, ScenarioError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tower_http::compression::CompressionLayer;

use crate::output::layout_json;
use crate::projects::{self, PROJECT_FILE, ProjectFile, Projects};
use crate::runner::{self, RunOptions};
use crate::thread_count;

#[derive(Args)]
pub struct ServeArgs {
    /// Data folder: `projects/` (one git repository per project) and `runs/`.
    /// Created if missing; an empty one gets the example projects.
    #[arg(long, default_value = "data")]
    data: PathBuf,
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
    projects::git(std::path::Path::new("."), &["--version"])
        .map_err(|e| format!("outliers-sim serve needs git to version projects: {e}"))?;
    let projects_dir = args.data.join("projects");
    let runs_dir = args.data.join("runs");
    for dir in [&projects_dir, &runs_dir] {
        fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    }
    let projects = Projects::new(projects_dir);
    if projects.ids().is_empty() {
        let created = projects::seed_examples(&projects)?;
        println!("created example projects: {}", created.join(", "));
    }
    let state = AppState::start(projects, runs_dir, thread_count(args.threads));
    let address = SocketAddr::new(args.host, args.port);
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(address)
            .await
            .map_err(|e| format!("can't listen on {address}: {e}"))?;
        println!("serving {} on http://{address}", args.data.display());
        axum::serve(listener, router(state))
            .with_graceful_shutdown(shutdown_signal())
            .await
            .map_err(|e| e.to_string())
    })
}

/// Ctrl+C, or SIGTERM (what systemd and Docker send to stop a service).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    println!("shutting down");
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(page("index.html")))
        .route("/build", get(page("build.html")))
        .route("/decide", get(page("decide.html")))
        .route("/common.css", get(asset("common.css")))
        .route("/common.js", get(asset("common.js")))
        .route("/build.js", get(asset("build.js")))
        .route("/decide.js", get(asset("decide.js")))
        .route("/api/v1/health", get(health))
        .route("/api/v1/projects", get(list_projects).post(create_project))
        .route("/api/v1/projects/{id}", get(get_project))
        .route("/api/v1/projects/{id}/check", post(check))
        .route("/api/v1/projects/{id}/save", post(save))
        .route("/api/v1/projects/{id}/publish", post(publish))
        .route("/api/v1/runs", get(list_runs).post(start_run))
        .route("/api/v1/runs/{id}", get(get_run))
        .route("/api/v1/runs/{id}/files/{name}", get(get_run_file))
        .layer(CompressionLayer::new())
        .with_state(state)
}

// --- web pages: plain HTML/CSS/JS compiled into the binary -------------------

const WEB: &[(&str, &str, &str)] = &[
    (
        "index.html",
        "text/html; charset=utf-8",
        include_str!("web/index.html"),
    ),
    (
        "build.html",
        "text/html; charset=utf-8",
        include_str!("web/build.html"),
    ),
    (
        "decide.html",
        "text/html; charset=utf-8",
        include_str!("web/decide.html"),
    ),
    (
        "common.css",
        "text/css; charset=utf-8",
        include_str!("web/common.css"),
    ),
    (
        "common.js",
        "text/javascript; charset=utf-8",
        include_str!("web/common.js"),
    ),
    (
        "build.js",
        "text/javascript; charset=utf-8",
        include_str!("web/build.js"),
    ),
    (
        "decide.js",
        "text/javascript; charset=utf-8",
        include_str!("web/decide.js"),
    ),
];

fn serve_web(name: &str) -> Response {
    let (_, media_type, body) = WEB.iter().find(|(n, _, _)| *n == name).expect("known file");
    (
        [
            (header::CONTENT_TYPE, *media_type),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        *body,
    )
        .into_response()
}

fn page(name: &'static str) -> impl Fn() -> std::future::Ready<Response> + Clone {
    move || std::future::ready(serve_web(name))
}

fn asset(name: &'static str) -> impl Fn() -> std::future::Ready<Response> + Clone {
    page(name)
}

// --- state and the run queue -------------------------------------------------

pub struct AppState {
    projects: Projects,
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
    label: String,
    project: String,
    /// `working` (the engineer's current model) or a published version.
    version: String,
    model: Model,
    layout: Option<Layout>,
    /// The exact model text that ran, kept with the results.
    model_text: String,
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
            "project": self.project,
            "version": self.version,
            "status": self.status,
            "progress": { "done": self.progress.load(Ordering::Relaxed), "total": self.total },
            "error": self.error,
            "files": self.files,
        })
    }
}

impl AppState {
    /// Creates the state and starts the worker thread that runs queued jobs.
    pub fn start(projects: Projects, runs: PathBuf, threads: usize) -> Arc<AppState> {
        let (queue, jobs_ready) = mpsc::channel::<String>();
        let state = Arc::new(AppState {
            projects,
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
        let result = runner::execute(
            &job.model,
            &RunOptions {
                replications: job.replications,
                threads: self.threads,
                log_replication: 1,
                layout: job.layout.as_ref(),
                progress: Some(&job.progress),
            },
            &out_dir,
        )
        .and_then(|outcome| {
            fs::write(out_dir.join("model.yaml"), &job.model_text)
                .map_err(|e| format!("can't write model.yaml: {e}"))?;
            Ok(outcome)
        });
        self.update(id, |j| match result {
            Ok(outcome) => {
                j.status = Status::Done;
                j.files = outcome
                    .files
                    .iter()
                    .filter_map(|f| f.file_name()?.to_str().map(String::from))
                    .chain(["model.yaml".to_string()])
                    .collect();
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

fn not_found_or_bad(message: String) -> Response {
    let status = if message.starts_with("no project") || message.contains("has no version") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::BAD_REQUEST
    };
    error(status, message)
}

/// A problem with a model: the change it comes from (if any), or the file and line.
fn problem(change: Option<&str>, file: Option<&str>, d: &Diagnostic) -> Value {
    json!({
        "change": change,
        "file": file,
        "line": d.position.map(|p| p.line),
        "column": d.position.map(|p| p.column),
        "message": d.message.strip_prefix("with this change: ").unwrap_or(&d.message),
    })
}

fn invalid(errors: Vec<Value>) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "ok": false, "error": "the model has errors", "errors": errors })),
    )
        .into_response()
}

// --- loading a project's model -------------------------------------------------

/// A project's model at a version, with changes applied and checked.
struct Loaded {
    project: ProjectFile,
    model: Model,
    /// The model as YAML: the file itself, or the file with the changes applied.
    text: String,
    layout: Option<Layout>,
}

/// Why a project's model couldn't be loaded.
enum LoadProblem {
    /// The request can't be served (no such project or version): a message.
    Request(String),
    /// The model has errors.
    Invalid(Vec<Value>),
}

impl LoadProblem {
    fn into_response(self) -> Response {
        match self {
            LoadProblem::Request(message) => not_found_or_bad(message),
            LoadProblem::Invalid(problems) => invalid(problems),
        }
    }
}

/// Loads `id` at `version` (`None`: the working copy) with `changes` applied.
fn load(
    state: &AppState,
    id: &str,
    version: Option<&str>,
    changes: &Map<String, Value>,
) -> Result<Loaded, LoadProblem> {
    let fail = |e: String| Err(LoadProblem::Request(e));
    let project = match state.projects.project_file(id, version) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    let base = match state.projects.read(id, version, &project.model) {
        Ok(text) => text,
        Err(e) => return fail(e),
    };
    let model_file = project.model.clone();
    let (model, text) = if changes.is_empty() {
        match outliers_sim_model::load_str(&base) {
            Ok(model) => (model, base),
            Err(e) => {
                return Err(LoadProblem::Invalid(
                    e.diagnostics()
                        .iter()
                        .map(|d| problem(None, Some(&model_file), d))
                        .collect(),
                ));
            }
        }
    } else {
        apply_changes(&base, &model_file, changes).map_err(LoadProblem::Invalid)?
    };
    let layout = match state
        .projects
        .read_optional(id, version, &project.layout_file())
    {
        Ok(None) => None,
        Ok(Some(source)) => match outliers_sim_model::load_layout(&source, &model) {
            Ok((layout, _warnings)) => Some(layout),
            Err(e) => {
                let file = project.layout_file();
                return Err(LoadProblem::Invalid(
                    e.diagnostics()
                        .iter()
                        .map(|d| problem(None, Some(&file), d))
                        .collect(),
                ));
            }
        },
        Err(e) => return fail(e),
    };
    Ok(Loaded {
        project,
        model,
        text,
        layout,
    })
}

/// Applies changes (dotted path -> value) to a model's source. Errors name
/// the change they come from, so a UI can mark the right field.
fn apply_changes(
    base: &str,
    model_file: &str,
    changes: &Map<String, Value>,
) -> Result<(Model, String), Vec<Value>> {
    let one = |message: String| vec![json!({ "change": null, "file": null, "message": message })];
    if let Some(key) = changes.keys().find(|k| {
        k.is_empty()
            || !k
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    }) {
        return Err(one(format!(
            "`{key}` is not a change path (names separated by dots)"
        )));
    }
    // The model keeps its own name: a scenario named after it.
    let name = outliers_sim_model::to_tree(base)
        .ok()
        .and_then(|t| t.get("name").and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| "model".into());
    let mut text = format!(
        "format: {}\nname: {}\nbase: {}\nchanges:\n",
        outliers_sim_model::SCENARIO_FORMAT,
        json!(name),
        json!(model_file)
    );
    let first_change_line = text.lines().count() as u64 + 1;
    for (path, value) in changes {
        text += &format!("  {path}: {value}\n");
    }
    let keys: Vec<&String> = changes.keys().collect();
    let located = |diagnostics: &[Diagnostic]| -> Vec<Value> {
        diagnostics
            .iter()
            .map(|d| {
                let change = d
                    .position
                    .and_then(|p| p.line.checked_sub(first_change_line))
                    .and_then(|i| keys.get(i as usize))
                    .map(|k| k.as_str());
                problem(
                    change,
                    None,
                    &Diagnostic {
                        position: None,
                        message: d.message.clone(),
                    },
                )
            })
            .collect()
    };
    let scenario =
        outliers_sim_model::parse_scenario(&text).map_err(|e| located(e.diagnostics()))?;
    match outliers_sim_model::load_scenario(&scenario, base) {
        Ok(model) => {
            let merged =
                outliers_sim_model::merge(&scenario, base).map_err(|e| located(e.diagnostics()))?;
            Ok((model, merged.text))
        }
        Err(ScenarioError::Scenario(e)) => Err(located(e.diagnostics())),
        Err(ScenarioError::Base(e)) => Err(e
            .diagnostics()
            .iter()
            .map(|d| problem(None, Some(model_file), d))
            .collect()),
    }
}

// --- health ----------------------------------------------------------------------

/// For load balancers and monitoring: 200 while the server and its run
/// queue are up.
async fn health(State(state): State<Arc<AppState>>) -> Response {
    let jobs = state.jobs.lock().expect("jobs lock");
    let count = |s: Status| jobs.iter().filter(|j| j.status == s).count();
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "runs": { "queued": count(Status::Queued), "running": count(Status::Running) },
    }))
    .into_response()
}

// --- projects --------------------------------------------------------------------

async fn list_projects(State(state): State<Arc<AppState>>) -> Response {
    let projects: Vec<Value> = state
        .projects
        .ids()
        .iter()
        .filter_map(|id| {
            let file = state.projects.project_file(id, None).ok()?;
            let latest = state.projects.latest_version(id).ok().flatten();
            Some(json!({
                "id": id,
                "name": file.name,
                "description": file.description,
                "published": latest.map(|v| json!({ "name": v.name, "date": v.date, "message": v.message })),
            }))
        })
        .collect();
    Json(json!({ "projects": projects })).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateProject {
    id: String,
    name: Option<String>,
    /// An existing project to start from (its working copy).
    from: String,
}

async fn create_project(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateProject>,
) -> Response {
    let source = match state.projects.project_file(&body.from, None) {
        Ok(p) => p,
        Err(e) => return not_found_or_bad(e),
    };
    let mut project = source.clone();
    project.name = body.name.clone().unwrap_or_else(|| body.id.clone());
    let mut files = vec![(
        PROJECT_FILE.to_string(),
        serde_saphyr::to_string(&project).unwrap_or_default(),
    )];
    for file in [source.model.clone(), source.layout_file()] {
        if let Ok(Some(text)) = state.projects.read_optional(&body.from, None, &file) {
            files.push((file, text));
        }
    }
    match state
        .projects
        .create(&body.id, &files, &format!("Created from {}", body.from))
    {
        Ok(()) => (StatusCode::CREATED, Json(json!({ "id": body.id }))).into_response(),
        Err(e) if e.contains("already exists") => error(StatusCode::CONFLICT, e),
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}

#[derive(Deserialize)]
struct VersionQuery {
    /// `latest`, `v3`, or absent for the engineer's working copy.
    version: Option<String>,
}

async fn get_project(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Query(query): Query<VersionQuery>,
) -> Response {
    let version = query.version.as_deref();
    let resolved = match version {
        None => None,
        Some(v) => match state.projects.version_tag(&id, v) {
            Ok(tag) => Some(tag),
            Err(e) => return not_found_or_bad(e),
        },
    };
    let project = match state.projects.project_file(&id, resolved.as_deref()) {
        Ok(p) => p,
        Err(e) => return not_found_or_bad(e),
    };
    let source = state
        .projects
        .read(&id, resolved.as_deref(), &project.model)
        .unwrap_or_default();
    let mut body = json!({
        "id": id,
        "name": project.name,
        "description": project.description,
        "version": resolved.clone().unwrap_or_else(|| "working".into()),
        "model_file": project.model,
        "source": source,
        "tree": outliers_sim_model::to_tree(&source).unwrap_or(Value::Null),
        "versions": state.projects.versions(&id).unwrap_or_default(),
    });
    if resolved.is_none() {
        body["history"] = json!(state.projects.history(&id).unwrap_or_default());
    }
    match load(&state, &id, resolved.as_deref(), &Map::new()) {
        Ok(loaded) => {
            body["valid"] = json!(true);
            body["errors"] = json!([]);
            body["layout"] = loaded.layout.as_ref().map_or(Value::Null, layout_json);
        }
        Err(LoadProblem::Request(e)) => return not_found_or_bad(e),
        Err(LoadProblem::Invalid(problems)) => {
            body["valid"] = json!(false);
            body["errors"] = json!(problems);
            body["layout"] = Value::Null;
        }
    }
    Json(body).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Changes {
    #[serde(default)]
    changes: Map<String, Value>,
    /// For saving: what changed, in a few words.
    #[serde(default)]
    message: String,
}

/// Checks pending changes against the working copy. Nothing is saved.
async fn check(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Json(body): Json<Changes>,
) -> Response {
    match load(&state, &id, None, &body.changes) {
        Ok(_) => Json(json!({ "ok": true, "errors": [] })).into_response(),
        Err(LoadProblem::Request(e)) => not_found_or_bad(e),
        Err(LoadProblem::Invalid(problems)) => {
            Json(json!({ "ok": false, "errors": problems })).into_response()
        }
    }
}

/// Applies pending changes to the working copy and saves a version (a commit).
async fn save(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Json(body): Json<Changes>,
) -> Response {
    if body.changes.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "nothing to save: there are no changes",
        );
    }
    let loaded = match load(&state, &id, None, &body.changes) {
        Ok(l) => l,
        Err(problem) => return problem.into_response(),
    };
    match state
        .projects
        .save(&id, &[(&loaded.project.model, &loaded.text)], &body.message)
    {
        Ok(commit) => Json(json!({ "ok": true, "saved": commit })).into_response(),
        Err(e) => error(StatusCode::CONFLICT, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publish {
    #[serde(default)]
    message: String,
}

/// Publishes the last saved version for managers. It must be a valid model.
async fn publish(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Json(body): Json<Publish>,
) -> Response {
    if let Err(problem) = load(&state, &id, None, &Map::new()) {
        return problem.into_response();
    }
    match state.projects.publish(&id, &body.message) {
        Ok(version) => Json(json!({ "ok": true, "published": version })).into_response(),
        Err(e) => error(StatusCode::CONFLICT, e),
    }
}

// --- runs ------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunRequest {
    project: String,
    /// `latest`, `v3`, or absent for the working copy.
    version: Option<String>,
    /// Changes to apply first (unsaved edits, or a scenario).
    #[serde(default)]
    changes: Map<String, Value>,
    /// Overrides the model's number of replications.
    replications: Option<u32>,
    /// A name for the run; defaults to the project and version.
    label: Option<String>,
}

async fn start_run(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RunRequest>,
) -> Response {
    if request.replications == Some(0) {
        return error(StatusCode::BAD_REQUEST, "`replications` must be at least 1");
    }
    let version = match request.version.as_deref() {
        None => None,
        Some(v) => match state.projects.version_tag(&request.project, v) {
            Ok(tag) => Some(tag),
            Err(e) => return not_found_or_bad(e),
        },
    };
    let loaded = match load(
        &state,
        &request.project,
        version.as_deref(),
        &request.changes,
    ) {
        Ok(l) => l,
        Err(problem) => return problem.into_response(),
    };
    let version_name = version.unwrap_or_else(|| "working".into());
    let label = request.label.clone().unwrap_or_else(|| {
        let changed = if request.changes.is_empty() {
            ""
        } else {
            " (with changes)"
        };
        format!("{} {version_name}{changed}", loaded.project.name)
    });
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let id = format!("{secs}-{}", state.next_id.fetch_add(1, Ordering::Relaxed));
    let job = Job {
        id: id.clone(),
        label,
        project: request.project.clone(),
        version: version_name,
        total: request
            .replications
            .unwrap_or(loaded.model.spec.experiment.replications),
        model: loaded.model,
        layout: loaded.layout,
        model_text: loaded.text,
        replications: request.replications,
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
    ("model.yaml", "text/yaml; charset=utf-8"),
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
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// A server over a fresh data folder with the example projects.
    fn app(name: &str) -> (Router, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("outliers-sim-serve-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("projects")).unwrap();
        let projects = Projects::new(root.join("projects"));
        projects::seed_examples(&projects).unwrap();
        let state = AppState::start(projects, root.join("runs"), 2);
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
    async fn lists_projects_with_their_published_version() {
        let (app, root) = app("list");
        let (status, body) = call(&app, "GET", "/api/v1/projects", None).await;
        assert_eq!(status, StatusCode::OK);
        let projects = body["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[1]["id"], "two-cells");
        assert_eq!(projects[1]["name"], "Two cells");
        assert_eq!(projects[1]["published"]["name"], "v1");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn engineer_flow_check_save_publish() {
        let (app, root) = app("flow");
        let (_, project) = call(&app, "GET", "/api/v1/projects/two-cells", None).await;
        assert_eq!(project["version"], "working");
        assert_eq!(project["valid"], true);
        assert_eq!(
            project["tree"]["sources"]["arrivals"]["output"]["rule"],
            "shortest_queue"
        );
        assert_eq!(project["layout"]["format"], "outliers-sim-layout/0.1");
        assert_eq!(project["history"].as_array().unwrap().len(), 1);

        // A bad change is reported against its path; nothing is saved.
        let bad = json!({ "changes": { "conveyors.c_a.speed": "5 min" } });
        let (_, checked) = call(
            &app,
            "POST",
            "/api/v1/projects/two-cells/check",
            Some(bad.clone()),
        )
        .await;
        assert_eq!(checked["ok"], false);
        assert_eq!(checked["errors"][0]["change"], "conveyors.c_a.speed");
        assert!(
            checked["errors"][0]["message"]
                .as_str()
                .unwrap()
                .contains("is a time, but a speed")
        );
        let (status, _) = call(&app, "POST", "/api/v1/projects/two-cells/save", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        // A good change is saved as a version, then published as v2.
        let good = json!({ "changes": { "servers.mill_b.capacity": 2 }, "message": "Second spindle on mill B" });
        let (status, saved) =
            call(&app, "POST", "/api/v1/projects/two-cells/save", Some(good)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["saved"]["message"], "Second spindle on mill B");
        let (_, project) = call(&app, "GET", "/api/v1/projects/two-cells", None).await;
        assert_eq!(project["tree"]["servers"]["mill_b"]["capacity"], 2);
        assert_eq!(
            project["tree"]["name"], "two-cells",
            "the model keeps its name"
        );
        let (status, published) = call(
            &app,
            "POST",
            "/api/v1/projects/two-cells/publish",
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{published}");
        assert_eq!(published["published"]["name"], "v2");
        let (status, _) = call(
            &app,
            "POST",
            "/api/v1/projects/two-cells/publish",
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        // Managers see published versions; v1 is unchanged.
        let (_, v1) = call(&app, "GET", "/api/v1/projects/two-cells?version=v1", None).await;
        assert_eq!(v1["version"], "v1");
        assert!(v1["tree"]["servers"]["mill_b"].get("capacity").is_none());
        let (_, latest) = call(
            &app,
            "GET",
            "/api/v1/projects/two-cells?version=latest",
            None,
        )
        .await;
        assert_eq!(latest["version"], "v2");
        assert!(latest.get("history").is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn runs_published_versions_and_working_copies_with_changes() {
        let (app, root) = app("runs");
        let (status, run) = call(
            &app,
            "POST",
            "/api/v1/runs",
            Some(json!({ "project": "two-cells", "version": "latest", "replications": 3 })),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{run}");
        assert_eq!(run["version"], "v1");
        let run = wait_for(&app, run["id"].as_str().unwrap()).await;
        assert_eq!(run["status"], "done", "{run}");
        assert_eq!(run["progress"], json!({ "done": 3, "total": 3 }));
        for file in [
            "report.html",
            "summary.json",
            "replications.csv",
            "events.jsonl",
            "layout.json",
            "model.yaml",
        ] {
            assert!(
                run["files"].as_array().unwrap().contains(&json!(file)),
                "{run}"
            );
        }

        let changes = json!({ "project": "two-cells", "changes": { "sources.arrivals.output": { "rule": "round_robin" } }, "replications": 2 });
        let (status, run) = call(&app, "POST", "/api/v1/runs", Some(changes)).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{run}");
        assert_eq!(run["label"], "Two cells working (with changes)");
        let run = wait_for(&app, run["id"].as_str().unwrap()).await;
        let id = run["id"].as_str().unwrap();
        let (_, model) = call(
            &app,
            "GET",
            &format!("/api/v1/runs/{id}/files/model.yaml"),
            None,
        )
        .await;
        assert!(
            model.as_str().unwrap().contains("rule: round_robin"),
            "the run keeps the exact model it ran"
        );

        let (status, body) = call(
            &app,
            "POST",
            "/api/v1/runs",
            Some(json!({ "project": "two-cells", "changes": { "experiment.replications": 0 } })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["errors"][0]["change"], "experiment.replications");
        let (status, _) = call(
            &app,
            "POST",
            "/api/v1/runs",
            Some(json!({ "project": "two-cells", "version": "v7" })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn creates_projects_from_existing_ones() {
        let (app, root) = app("create");
        let body = json!({ "id": "line-2", "name": "Second line", "from": "conveyor-line" });
        let (status, _) = call(&app, "POST", "/api/v1/projects", Some(body.clone())).await;
        assert_eq!(status, StatusCode::CREATED);
        let (_, project) = call(&app, "GET", "/api/v1/projects/line-2", None).await;
        assert_eq!(project["name"], "Second line");
        assert_eq!(project["valid"], true);
        assert!(project["layout"].is_object());
        assert!(
            project["versions"].as_array().unwrap().is_empty(),
            "not published yet"
        );
        let (status, _) = call(&app, "POST", "/api/v1/projects", Some(body)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn rejects_bad_ids_and_unknown_files() {
        let (app, root) = app("ids");
        for uri in [
            "/api/v1/projects/..%2Fetc",
            "/api/v1/projects/Two-Cells",
            "/api/v1/projects/nope",
        ] {
            let (status, _) = call(&app, "GET", uri, None).await;
            assert!(
                status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
                "{uri}: {status}"
            );
        }
        let (status, _) = call(&app, "GET", "/api/v1/runs/nope/files/report.html", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = call(&app, "GET", "/api/v1/runs/nope/files/secret.txt", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn health() {
        let (app, root) = app("health");
        let (status, body) = call(&app, "GET", "/api/v1/health", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn serves_the_three_pages() {
        let (app, root) = app("pages");
        for (uri, marker) in [
            ("/", "href=\"build\""),
            ("/build", "<script src=\"build.js\">"),
            ("/decide", "<script src=\"decide.js\">"),
            ("/common.js", "const API"),
        ] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            let body = response.into_body().collect().await.unwrap().to_bytes();
            assert!(String::from_utf8_lossy(&body).contains(marker), "{uri}");
        }
        fs::remove_dir_all(root).unwrap();
    }
}
