//! `simulancer serve`: the web application and its HTTP API.
//!
//! Three audiences, three interfaces over the same API:
//! - engineers build and version models (`/modeling`),
//! - managers compare scenarios on published versions (`/simulation`),
//! - developers use the HTTP API directly (docs/api.md).
//!
//! The web pages use only the public API. Simulations run on the server, one
//! run at a time on all cores; later runs wait in a queue. Projects are git
//! repositories in `<data>/projects`; managers' scenarios are in
//! `<data>/scenarios`; each run's results go to `<data>/runs/<id>`, with a
//! `run.json` that brings the run back after a restart, and each comparison to
//! `<data>/runs/comparisons/<id>.json`.

use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
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
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use simulancer_model::{Diagnostic, Layout, Model, ScenarioError};
use tower_http::compression::CompressionLayer;

use crate::compare::compare_pair;
use crate::decisions::{self, Exposed, Target};
use crate::markdown;
use crate::measures::{self, Measure};
use crate::output::layout_json;
use crate::plain;
use crate::precision::KeyResult;
use crate::projects::{self, DOCUMENTATION_FILE, PROJECT_FILE, ProjectFile, Projects};
use crate::roles::{Role, Roles, decider_may};
use crate::runner::{self, RunOptions};
use crate::scenarios::Scenarios;
use crate::thread_count;

#[derive(Args)]
pub struct ServeArgs {
    /// Data folder: `projects/` (one git repository per project), `scenarios/`
    /// (managers' scenarios) and `runs/`.
    /// Created if missing. A new one starts with no projects.
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
    /// Turns roles on: the header the reverse proxy sets with the user's
    /// groups (e.g. `X-Forwarded-Groups`). Users in a modeler group can build
    /// and publish models; everyone else only builds and compares scenarios.
    /// Without it, everyone can do everything.
    #[arg(long)]
    role_header: Option<String>,
    /// The groups, in the role header, of people who build models
    /// (comma-separated).
    #[arg(long, default_value = "modeler", value_delimiter = ',')]
    modelers: Vec<String>,
}

/// A run's folder name: `<seconds>-<n>`. Returns `n`.
fn run_number(name: &str) -> Option<u64> {
    let (a, b) = name.split_once('-')?;
    (!a.is_empty() && a.chars().all(|c| c.is_ascii_digit()))
        .then(|| b.parse().ok())
        .flatten()
}

/// The finished runs and the comparisons an earlier start left in `runs`, and
/// the next free id number. A run folder without a readable `run.json` (it was
/// queued or running when the server stopped) is removed; anything not named
/// like a run is left alone.
fn restore(runs: &Path, projects: &Projects) -> (Vec<Job>, Vec<ComparisonJob>, u64) {
    let mut jobs = Vec::new();
    let mut removed = 0;
    let mut next = 1;
    for entry in fs::read_dir(runs).into_iter().flatten().flatten() {
        let path = entry.path();
        let Some(n) = entry.file_name().to_str().and_then(run_number) else {
            continue;
        };
        if !path.is_dir() {
            continue;
        }
        match read_run(&path) {
            Some(job) => {
                next = next.max(n + 1);
                jobs.push((job.queued_at, n, job));
            }
            None => {
                if fs::remove_dir_all(&path).is_ok() {
                    removed += 1;
                }
            }
        }
    }
    if removed > 0 {
        let runs = if removed == 1 { "run" } else { "runs" };
        println!("removed {removed} unfinished {runs} from an earlier start");
    }
    jobs.sort_by_key(|(at, n, _)| (*at, *n));
    let jobs: Vec<Job> = jobs.into_iter().map(|(_, _, job)| job).collect();

    let mut comparisons = Vec::new();
    for entry in fs::read_dir(runs.join(COMPARISONS_DIR))
        .into_iter()
        .flatten()
        .flatten()
    {
        let Some(stored) = fs::read_to_string(entry.path())
            .ok()
            .and_then(|text| serde_json::from_str::<StoredComparison>(&text).ok())
        else {
            continue;
        };
        let Some(n) = stored
            .id
            .strip_prefix('c')
            .and_then(|n| n.parse::<u64>().ok())
        else {
            continue;
        };
        // What the version exposes is read again; a deleted project's comparisons stay out.
        let Ok((exposed, _)) = exposed_in(projects, &stored.project, Some(&stored.version)) else {
            continue;
        };
        next = next.max(n + 1);
        comparisons.push((
            n,
            ComparisonJob {
                id: stored.id,
                project: stored.project,
                version: stored.version,
                exposed,
                items: stored.items,
            },
        ));
    }
    comparisons.sort_by_key(|(n, _)| *n);
    let comparisons = comparisons.into_iter().map(|(_, c)| c).collect();
    (jobs, comparisons, next)
}

/// A finished run as its `run.json`, model and results describe it.
fn read_run(dir: &Path) -> Option<Job> {
    let stored: StoredRun =
        serde_json::from_str(&fs::read_to_string(dir.join(RUN_RECORD)).ok()?).ok()?;
    let model_text = fs::read_to_string(dir.join("model.yaml")).ok()?;
    let model = simulancer_model::load_str(&model_text).ok()?;
    let (measures, done) = match stored.status {
        Status::Done => (
            Some(Arc::new(read_measures(
                &model,
                &dir.join("replications.csv"),
            )?)),
            stored.total,
        ),
        Status::Failed => (None, 0),
        Status::Queued | Status::Running => return None,
    };
    Some(Job {
        id: stored.id,
        label: stored.label,
        project: stored.project,
        version: stored.version,
        model,
        layout: None,
        model_text,
        replications: stored.replications,
        total: stored.total,
        progress: Arc::new(AtomicU32::new(done)),
        status: stored.status,
        error: stored.error,
        files: stored.files,
        queued_at: stored.queued_at,
        measures,
    })
}

/// A run's measures, one value per replication, from its `replications.csv`
/// (values there are written exactly, so comparisons come out the same).
fn read_measures(model: &Model, csv: &Path) -> Option<Vec<Measure>> {
    let mut measures = measures::collect(&model.spec, &model.outputs, &[]);
    let text = fs::read_to_string(csv).ok()?;
    for line in text.lines().skip(1) {
        let [replication, object_type, object, name, _, value] =
            <[&str; 6]>::try_from(line.split(',').collect::<Vec<_>>()).ok()?;
        let measure = measures
            .iter_mut()
            .find(|m| m.object_type == object_type && m.object == object && m.name == name)?;
        if replication.parse::<usize>().ok()? != measure.values.len() + 1 {
            return None;
        }
        measure.values.push(if value.is_empty() {
            f64::NAN
        } else {
            value.parse().ok()?
        });
    }
    Some(measures)
}

pub fn serve(args: &ServeArgs) -> Result<(), String> {
    projects::git(std::path::Path::new("."), &["--version"])
        .map_err(|e| format!("simulancer serve needs git to version projects: {e}"))?;
    let projects_dir = args.data.join("projects");
    let runs_dir = args.data.join("runs");
    // A new data folder starts empty: projects start from templates or blank.
    for dir in [&projects_dir, &runs_dir] {
        fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    }
    let projects = Projects::new(projects_dir);
    let scenarios = Scenarios::new(args.data.join("scenarios"));
    let roles = match &args.role_header {
        None => None,
        Some(name) => Some(Roles {
            header: name
                .parse()
                .map_err(|_| format!("`{name}` isn't a valid header name"))?,
            modelers: args
                .modelers
                .iter()
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty())
                .collect(),
        }),
    };
    if let Some(r) = &roles {
        println!(
            "roles on: modelers are the groups {} in {}",
            r.modelers.join(", "),
            r.header
        );
    }
    let state = AppState::start(projects, scenarios, runs_dir, thread_count(args.threads))
        .with_roles(roles);
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
        .route("/modeling", get(page("modeling.html")))
        .route("/simulation", get(page("simulation.html")))
        .route("/common.css", get(asset("common.css")))
        .route("/art.js", get(asset("art.js")))
        .route("/common.js", get(asset("common.js")))
        .route("/theme.js", get(asset("theme.js")))
        .route("/index.js", get(asset("index.js")))
        .route("/modeling.js", get(asset("modeling.js")))
        .route("/simulation.js", get(asset("simulation.js")))
        .route("/favicon-32.png", get(asset("favicon-32.png")))
        .route("/favicon.ico", get(asset("favicon-32.png")))
        .route("/icon-180.png", get(asset("icon-180.png")))
        .route("/api/v1/health", get(health))
        .route("/api/v1/me", get(me))
        .route("/api/v1/projects", get(list_projects).post(create_project))
        .route("/api/v1/templates", get(list_templates))
        .route(
            "/api/v1/projects/{id}",
            get(get_project)
                .patch(update_project)
                .delete(delete_project),
        )
        .route("/api/v1/projects/{id}/archive", post(archive_project))
        .route("/api/v1/projects/{id}/restore", post(restore_project))
        .route("/api/v1/projects/{id}/check", post(check))
        .route("/api/v1/projects/{id}/save", post(save))
        .route("/api/v1/projects/{id}/publish", post(publish))
        .route(
            "/api/v1/projects/{id}/scenarios",
            get(list_scenarios).post(create_scenario),
        )
        .route(
            "/api/v1/projects/{id}/scenarios/{scenario}",
            axum::routing::put(update_scenario).delete(delete_scenario),
        )
        .route(
            "/api/v1/comparisons",
            get(list_comparisons).post(start_comparison),
        )
        .route("/api/v1/comparisons/{id}", get(get_comparison))
        .route("/api/v1/runs", get(list_runs).post(start_run))
        .route("/api/v1/runs/{id}", get(get_run).delete(delete_run))
        .route("/api/v1/runs/{id}/files/{name}", get(get_run_file))
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            enforce_roles,
        ))
        .layer(CompressionLayer::new())
        .with_state(state)
}

/// With roles on, keeps deciders to what they may do, and tells handlers
/// the role of each request.
async fn enforce_roles(
    State(state): State<Arc<AppState>>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let role = state.role(request.headers());
    if role == Role::Decider && !decider_may(request.method(), request.uri().path()) {
        if request.uri().path() == "/modeling" {
            return (
                StatusCode::FORBIDDEN,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                "<!doctype html><meta charset=utf-8><title>SimuLancer</title>\
                 <p style=\"font: 16px system-ui; margin: 40px\">The Modeling page is for the people who build the models. \
                 You can compare scenarios of the published versions on the <a href=\"./\">projects' Simulation pages</a>.</p>",
            )
                .into_response();
        }
        return error(
            StatusCode::FORBIDDEN,
            "this needs a modeler: people who decide build and compare scenarios of published versions",
        );
    }
    request.extensions_mut().insert(role);
    next.run(request).await
}

/// The role of whoever asks, so the pages show only what they may do.
async fn me(State(state): State<Arc<AppState>>, headers: axum::http::HeaderMap) -> Response {
    let roles = state.roles.lock().expect("roles lock").is_some();
    Json(json!({ "role": state.role(&headers), "roles": roles })).into_response()
}

// --- web pages: plain HTML/CSS/JS and the icon, compiled into the binary -----

const WEB: &[(&str, &str, &[u8])] = &[
    (
        "index.html",
        "text/html; charset=utf-8",
        include_bytes!("web/index.html"),
    ),
    (
        "modeling.html",
        "text/html; charset=utf-8",
        include_bytes!("web/modeling.html"),
    ),
    (
        "simulation.html",
        "text/html; charset=utf-8",
        include_bytes!("web/simulation.html"),
    ),
    (
        "common.css",
        "text/css; charset=utf-8",
        include_bytes!("web/common.css"),
    ),
    (
        "art.js",
        "text/javascript; charset=utf-8",
        include_bytes!("web/art.js"),
    ),
    (
        "common.js",
        "text/javascript; charset=utf-8",
        include_bytes!("web/common.js"),
    ),
    (
        "theme.js",
        "text/javascript; charset=utf-8",
        include_bytes!("web/theme.js"),
    ),
    (
        "index.js",
        "text/javascript; charset=utf-8",
        include_bytes!("web/index.js"),
    ),
    (
        "modeling.js",
        "text/javascript; charset=utf-8",
        include_bytes!("web/modeling.js"),
    ),
    (
        "simulation.js",
        "text/javascript; charset=utf-8",
        include_bytes!("web/simulation.js"),
    ),
    (
        "favicon-32.png",
        "image/png",
        include_bytes!("web/favicon-32.png"),
    ),
    (
        "icon-180.png",
        "image/png",
        include_bytes!("web/icon-180.png"),
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
    scenarios: Scenarios,
    runs: PathBuf,
    threads: usize,
    jobs: Mutex<Vec<Job>>,
    comparisons: Mutex<Vec<ComparisonJob>>,
    queue: mpsc::Sender<String>,
    next_id: AtomicU64,
    /// Who may change models, when the proxy says (`--role-header`).
    roles: Mutex<Option<Roles>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    /// When it was asked for, in seconds since 1970 (shown as a date).
    queued_at: u64,
    /// Once done: every measure, one value per replication (for comparisons).
    measures: Option<Arc<Vec<Measure>>>,
}

/// A finished run's file that brings it back after a restart.
const RUN_RECORD: &str = "run.json";
/// Where comparisons are kept, inside the runs folder.
const COMPARISONS_DIR: &str = "comparisons";

/// What `run.json` keeps of a run; the model and the measures are read back
/// from `model.yaml` and `replications.csv` next to it.
#[derive(Serialize, Deserialize)]
struct StoredRun {
    id: String,
    label: String,
    project: String,
    version: String,
    replications: Option<u32>,
    total: u32,
    status: Status,
    error: Option<String>,
    files: Vec<String>,
    queued_at: u64,
}

impl Job {
    fn stored(&self) -> StoredRun {
        StoredRun {
            id: self.id.clone(),
            label: self.label.clone(),
            project: self.project.clone(),
            version: self.version.clone(),
            replications: self.replications,
            total: self.total,
            status: self.status,
            error: self.error.clone(),
            files: self.files.clone(),
            queued_at: self.queued_at,
        }
    }

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
            "queued_at": self.queued_at,
        })
    }
}

impl AppState {
    /// Creates the state and starts the worker thread that runs queued jobs.
    pub fn start(
        projects: Projects,
        scenarios: Scenarios,
        runs: PathBuf,
        threads: usize,
    ) -> Arc<AppState> {
        let (queue, jobs_ready) = mpsc::channel::<String>();
        let (jobs, comparisons, next_id) = restore(&runs, &projects);
        let state = Arc::new(AppState {
            projects,
            scenarios,
            runs,
            threads,
            jobs: Mutex::new(jobs),
            comparisons: Mutex::new(comparisons),
            queue,
            next_id: AtomicU64::new(next_id),
            roles: Mutex::new(None),
        });
        let worker = Arc::clone(&state);
        thread::spawn(move || {
            for id in jobs_ready {
                worker.run_job(&id);
            }
        });
        state
    }

    /// Whether a run of the project is queued or running.
    fn has_unfinished_runs(&self, project: &str) -> bool {
        self.jobs
            .lock()
            .expect("jobs lock")
            .iter()
            .any(|j| j.project == project && matches!(j.status, Status::Queued | Status::Running))
    }

    /// Removes the finished runs and the comparisons of a project that no
    /// longer exists, from the lists and from disk, so a new project with the
    /// same id starts with none. Comparison files are read from disk, since a
    /// restart leaves an archived project's comparisons out of the list.
    fn forget_project(&self, project: &str) -> Result<(), String> {
        self.comparisons
            .lock()
            .expect("comparisons lock")
            .retain(|c| c.project != project);
        let runs: Vec<String> = {
            let mut jobs = self.jobs.lock().expect("jobs lock");
            let (gone, kept) = std::mem::take(&mut *jobs).into_iter().partition(|j| {
                j.project == project && matches!(j.status, Status::Done | Status::Failed)
            });
            *jobs = kept;
            gone.into_iter().map(|j: Job| j.id).collect()
        };
        let mut problems = Vec::new();
        for id in runs {
            match fs::remove_dir_all(self.runs.join(&id)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    problems.push(format!("run `{id}`: {e}"));
                }
                _ => {}
            }
        }
        for entry in fs::read_dir(self.runs.join(COMPARISONS_DIR))
            .into_iter()
            .flatten()
            .flatten()
        {
            let theirs = fs::read_to_string(entry.path())
                .ok()
                .and_then(|text| serde_json::from_str::<StoredComparison>(&text).ok())
                .is_some_and(|c| c.project == project);
            if theirs && let Err(e) = fs::remove_file(entry.path()) {
                problems.push(format!("{}: {e}", entry.path().display()));
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "can't remove all of project `{project}`'s results: {}",
                problems.join("; ")
            ))
        }
    }

    /// Turns roles on (or off, with None).
    pub fn with_roles(self: Arc<Self>, roles: Option<Roles>) -> Arc<Self> {
        *self.roles.lock().expect("roles lock") = roles;
        self
    }

    fn role(&self, headers: &axum::http::HeaderMap) -> Role {
        self.roles
            .lock()
            .expect("roles lock")
            .as_ref()
            .map_or(Role::Modeler, |r| r.role(headers))
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
        // The results the project shows on the Simulation page are key results too.
        let version = (job.version != "working").then_some(job.version.as_str());
        let key_results: Vec<KeyResult> = exposed_in(&self.projects, &job.project, version)
            .map(|(exposed, _)| exposed.results)
            .unwrap_or_default()
            .into_iter()
            .map(|r| KeyResult {
                label: r.label,
                object: r.object,
                measure: r.measure,
            })
            .collect();
        let result = runner::execute(
            &job.model,
            &RunOptions {
                replications: job.replications,
                threads: self.threads,
                log_replication: 1,
                layout: job.layout.as_ref(),
                progress: Some(&job.progress),
                key_results: &key_results,
            },
            &out_dir,
        )
        .and_then(|outcome| {
            fs::write(out_dir.join("model.yaml"), &job.model_text)
                .map_err(|e| format!("can't write model.yaml: {e}"))?;
            Ok(outcome)
        });
        if result.is_err() {
            // A failed run keeps its model too, so it can be listed after a restart.
            let _ = fs::create_dir_all(&out_dir)
                .and_then(|()| fs::write(out_dir.join("model.yaml"), &job.model_text));
        }
        self.update(id, |j| match result {
            Ok(outcome) => {
                j.status = Status::Done;
                j.files = outcome
                    .files
                    .iter()
                    .filter_map(|f| f.file_name()?.to_str().map(String::from))
                    .chain(["model.yaml".to_string()])
                    .collect();
                j.measures = Some(Arc::new(
                    outcome.measures.into_iter().map(|(m, _)| m).collect(),
                ));
            }
            Err(error) => {
                j.status = Status::Failed;
                j.error = Some(error);
            }
        });
        let record = self
            .jobs
            .lock()
            .expect("jobs lock")
            .iter()
            .find(|j| j.id == id)
            .map(Job::stored);
        if let Some(record) = record {
            let text = serde_json::to_string_pretty(&record).expect("a run record is plain data");
            if let Err(e) = fs::write(out_dir.join(RUN_RECORD), text) {
                eprintln!("run {id} won't be kept after a restart: can't write {RUN_RECORD}: {e}");
            }
        }
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
    load_with(state, id, version, changes, None)
}

/// Like [`load`], with a whole new layout (as JSON) instead of the saved one.
fn load_with(
    state: &AppState,
    id: &str,
    version: Option<&str>,
    changes: &Map<String, Value>,
    layout: Option<&Value>,
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
        match simulancer_model::load_str(&base) {
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
    let layout_source = match layout {
        Some(tree) => Ok(Some(layout_text(tree, &project.model))),
        None => state
            .projects
            .read_optional(id, version, &project.layout_file()),
    };
    let layout = match layout_source {
        Ok(None) => None,
        Ok(Some(source)) => match simulancer_model::load_layout(&source, &model) {
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

/// A layout given as JSON (as `layout.json` or the file's own shape) as
/// layout file text.
fn layout_text(tree: &Value, model_file: &str) -> String {
    let mut tree = tree.clone();
    if let Some(map) = tree.as_object_mut() {
        map.remove("format");
        map.insert("model".into(), json!(model_file));
        map.entry("units").or_insert(json!("m"));
        // Drop empty fields (`"model": null` on objects), and a fleet with
        // nothing to say.
        for section in ["objects", "conveyors", "vehicles", "entities", "resources"] {
            if let Some(items) = map.get_mut(section).and_then(Value::as_object_mut) {
                for item in items.values_mut().filter_map(Value::as_object_mut) {
                    item.retain(|_, v| !v.is_null());
                }
                items.retain(|_, v| v.as_object().is_none_or(|o| !o.is_empty()));
            }
        }
        for section in ["vehicles", "entities", "resources"] {
            if map
                .get(section)
                .and_then(Value::as_object)
                .is_some_and(|v| v.is_empty())
            {
                map.remove(section);
            }
        }
    }
    simulancer_model::to_yaml(&tree)
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
    let name = simulancer_model::to_tree(base)
        .ok()
        .and_then(|t| t.get("name").and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| "model".into());
    let mut text = format!(
        "format: {}\nname: {}\nbase: {}\nchanges:\n",
        simulancer_model::SCENARIO_FORMAT,
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
    let scenario = simulancer_model::parse_scenario(&text).map_err(|e| located(e.diagnostics()))?;
    match simulancer_model::load_scenario(&scenario, base) {
        Ok(model) => {
            let merged =
                simulancer_model::merge(&scenario, base).map_err(|e| located(e.diagnostics()))?;
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

/// What a project exposes to managers at a version, and the problems with it.
fn exposed(
    state: &AppState,
    id: &str,
    version: Option<&str>,
) -> Result<(Exposed, Vec<String>), String> {
    exposed_in(&state.projects, id, version)
}

fn exposed_in(
    projects: &Projects,
    id: &str,
    version: Option<&str>,
) -> Result<(Exposed, Vec<String>), String> {
    let project = projects.project_file(id, version)?;
    let source = projects.read(id, version, &project.model)?;
    let (exposed, problems, _) = read_exposed(&project, &source);
    Ok((exposed, problems))
}

/// What a project file exposes for a model's source, the problems with it,
/// and the measures the model reports (none if the model has errors).
fn read_exposed(project: &ProjectFile, source: &str) -> (Exposed, Vec<String>, Vec<Measure>) {
    let model = simulancer_model::load_str(source).ok();
    let measures = model
        .as_ref()
        .map(|m| measures::collect(&m.spec, &m.outputs, &[]));
    let try_changes = |changes: &Map<String, Value>| -> Result<(), String> {
        apply_changes(source, &project.model, changes)
            .map(|_| ())
            .map_err(|errors| {
                errors
                    .iter()
                    .map(|e| match (e["change"].as_str(), e["message"].as_str()) {
                        (Some(change), Some(message)) => format!("`{change}`: {message}"),
                        (None, Some(message)) => message.to_string(),
                        _ => e.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            })
    };
    let words = model.as_ref().map(|m| m.words.clone()).unwrap_or_default();
    let target = measures.as_ref().map(|measures| Target {
        source,
        measures,
        try_changes: &try_changes,
        words: &words,
    });
    let (exposed, problems) =
        decisions::read(&project.decisions, &project.results, target.as_ref());
    (exposed, problems, measures.unwrap_or_default())
}

/// The measures a model reports, for picking results.
fn measures_json(measures: &[Measure]) -> Value {
    json!(
        measures
            .iter()
            .map(|m| json!({ "object_type": m.object_type, "object": m.object, "measure": m.name, "unit": m.unit }))
            .collect::<Vec<_>>()
    )
}

/// Problems in project.yaml, in the shape of model errors.
fn project_problems(problems: &[String]) -> Vec<Value> {
    problems
        .iter()
        .map(|message| json!({ "change": null, "file": PROJECT_FILE, "line": null, "column": null, "message": message }))
        .collect()
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

#[derive(Deserialize)]
struct ListProjectsQuery {
    #[serde(default)]
    archived: bool,
}

async fn list_projects(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListProjectsQuery>,
) -> Response {
    if query.archived {
        let projects: Vec<Value> = state
            .projects
            .archived_ids()
            .iter()
            .filter_map(|id| {
                let file = state.projects.archived_project_file(id).ok()?;
                Some(json!({ "id": id, "name": file.name, "description": file.description }))
            })
            .collect();
        return Json(json!({ "projects": projects })).into_response();
    }
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
struct ProjectDetails {
    name: Option<String>,
    /// Empty removes the description.
    description: Option<String>,
}

async fn update_project(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Json(body): Json<ProjectDetails>,
) -> Response {
    match state
        .projects
        .set_details(&id, body.name.as_deref(), body.description.as_deref())
    {
        Ok(saved) => {
            let file = match state.projects.project_file(&id, None) {
                Ok(f) => f,
                Err(e) => return not_found_or_bad(e),
            };
            Json(json!({ "id": id, "name": file.name, "description": file.description, "saved": saved }))
                .into_response()
        }
        Err(e) => not_found_or_bad(e),
    }
}

async fn archive_project(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
) -> Response {
    match state.projects.archive(&id) {
        Ok(()) => Json(json!({ "id": id, "archived": true })).into_response(),
        Err(e) if e.contains("already exists") => error(StatusCode::CONFLICT, e),
        Err(e) => not_found_or_bad(e),
    }
}

async fn restore_project(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
) -> Response {
    match state.projects.restore(&id) {
        Ok(()) => Json(json!({ "id": id, "archived": false })).into_response(),
        Err(e) if e.contains("already exists") => error(StatusCode::CONFLICT, e),
        Err(e) if e.starts_with("no archived project") => error(StatusCode::NOT_FOUND, e),
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}

/// Deletes an archived project with its scenarios, runs and comparisons. A
/// project on the list has to be archived first, and its runs finished.
async fn delete_project(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
) -> Response {
    if state.projects.dir(&id).is_ok() {
        return error(
            StatusCode::CONFLICT,
            format!("project `{id}` isn't archived; archive it before deleting it"),
        );
    }
    if state.has_unfinished_runs(&id) {
        return error(
            StatusCode::CONFLICT,
            format!("project `{id}` has runs that haven't finished; delete it when they're done"),
        );
    }
    match state.projects.delete_archived(&id) {
        Ok(()) => match state
            .scenarios
            .remove_all(&id)
            .and_then(|()| state.forget_project(&id))
        {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
        },
        Err(e) if e.starts_with("no archived project") => error(StatusCode::NOT_FOUND, e),
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateProject {
    id: String,
    name: Option<String>,
    /// An existing project to start from (its working copy). Without it
    /// or `template`, the project starts blank: a simple line to build on.
    from: Option<String>,
    /// A template to start from (`GET /api/v1/templates`); the project
    /// starts with it saved and published as v1.
    template: Option<String>,
}

/// The templates a project can start from.
async fn list_templates() -> Response {
    let templates: Vec<Value> = projects::TEMPLATES
        .iter()
        .map(|t| {
            let file = t.project_file();
            json!({ "id": t.id, "name": file.name, "description": file.description, "tags": t.tags() })
        })
        .collect();
    Json(json!({ "templates": templates })).into_response()
}

async fn create_project(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateProject>,
) -> Response {
    if let Some(template) = &body.template {
        if body.from.is_some() {
            return error(
                StatusCode::BAD_REQUEST,
                "give `from` or `template`, not both",
            );
        }
        if !projects::TEMPLATES.iter().any(|t| t.id == template) {
            return error(StatusCode::NOT_FOUND, format!("no template `{template}`"));
        }
        return match projects::create_from_template(
            &state.projects,
            &body.id,
            template,
            body.name.as_deref(),
        ) {
            Ok(()) => created(&state, &body.id),
            Err(e) if e.contains("already exists") => error(StatusCode::CONFLICT, e),
            Err(e) => error(StatusCode::BAD_REQUEST, e),
        };
    }
    let name = body.name.clone().unwrap_or_else(|| body.id.clone());
    let (files, message) = match &body.from {
        None => (
            projects::blank(&body.id, &name),
            "Started blank".to_string(),
        ),
        Some(from) => {
            let source = match state.projects.project_file(from, None) {
                Ok(p) => p,
                Err(e) => return not_found_or_bad(e),
            };
            let mut project = source.clone();
            project.name = name;
            let mut files = vec![(PROJECT_FILE.to_string(), project.to_text())];
            for file in [
                source.model.clone(),
                source.layout_file(),
                DOCUMENTATION_FILE.into(),
            ] {
                if let Ok(Some(text)) = state.projects.read_optional(from, None, &file) {
                    files.push((file, text));
                }
            }
            (files, format!("Created from {from}"))
        }
    };
    match state.projects.create(&body.id, &files, &message) {
        Ok(()) => created(&state, &body.id),
        Err(e) if e.contains("already exists") => error(StatusCode::CONFLICT, e),
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}

/// A new project's answer. Results still kept under its id belong to a project
/// deleted before deleting took them along, so they go.
fn created(state: &AppState, id: &str) -> Response {
    if let Err(e) = state.forget_project(id) {
        eprintln!("{e}");
    }
    (StatusCode::CREATED, Json(json!({ "id": id }))).into_response()
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
    // The name and description are the project's, whichever version is shown.
    let current = match &resolved {
        Some(_) => state
            .projects
            .project_file(&id, None)
            .unwrap_or_else(|_| project.clone()),
        None => project.clone(),
    };
    let mut body = json!({
        "id": id,
        "name": current.name,
        "description": current.description,
        "version": resolved.clone().unwrap_or_else(|| "working".into()),
        "model_file": project.model,
        "source": source,
        "tree": simulancer_model::to_tree(&source).unwrap_or(Value::Null),
        "versions": state.projects.versions(&id).unwrap_or_default(),
    });
    if resolved.is_none() {
        body["history"] = json!(state.projects.history(&id).unwrap_or_default());
    }
    let documentation = state
        .projects
        .read_optional(&id, resolved.as_deref(), DOCUMENTATION_FILE)
        .ok()
        .flatten()
        .unwrap_or_default();
    body["documentation_html"] = json!(markdown::to_html(&documentation));
    body["documentation"] = json!(documentation);
    let (exposed, problems, measures) = read_exposed(&project, &source);
    body["project_file"] = json!(project);
    body["measures"] = measures_json(&measures);
    body["decisions"] = json!(exposed.decisions);
    body["results"] = json!(exposed.results);
    body["decision_problems"] = json!(problems);
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
    /// New `decisions` for project.yaml (replacing the saved ones).
    decisions: Option<Map<String, Value>>,
    /// New `results` for project.yaml.
    results: Option<Vec<Value>>,
    /// A whole new layout (the `*.layout.yaml` file as JSON).
    layout: Option<Value>,
    /// New documentation (`README.md`, in Markdown); empty removes it.
    documentation: Option<String>,
    /// For saving: what changed, in a few words.
    #[serde(default)]
    message: String,
}

impl Changes {
    /// The project file with the pending decisions and results.
    fn apply_to(&self, project: &mut ProjectFile) {
        if let Some(decisions) = &self.decisions {
            project.decisions = decisions.clone();
        }
        if let Some(results) = &self.results {
            project.results = results.clone();
        }
    }

    fn touches_project_file(&self) -> bool {
        self.decisions.is_some() || self.results.is_some()
    }
}

/// Checks pending changes (to the model, and to what managers may change)
/// against the working copy. Nothing is saved.
async fn check(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Json(body): Json<Changes>,
) -> Response {
    let (ok, errors, mut project, source) =
        match load_with(&state, &id, None, &body.changes, body.layout.as_ref()) {
            Ok(loaded) => (true, Vec::new(), loaded.project, loaded.text),
            Err(LoadProblem::Request(e)) => return not_found_or_bad(e),
            Err(LoadProblem::Invalid(problems)) => {
                let project = match state.projects.project_file(&id, None) {
                    Ok(p) => p,
                    Err(e) => return not_found_or_bad(e),
                };
                // The model has errors: only the structure of decisions is checked.
                (false, problems, project, String::new())
            }
        };
    body.apply_to(&mut project);
    let (exposed, problems, _) = read_exposed(&project, &source);
    let mut response = json!({
        "ok": ok,
        "errors": errors,
        "decisions": exposed.decisions,
        "results": exposed.results,
        "decision_problems": problems,
    });
    if let Some(text) = &body.documentation {
        response["documentation_html"] = json!(markdown::to_html(text));
    }
    Json(response).into_response()
}

/// Applies pending changes to the working copy and saves a version (a commit).
async fn save(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Json(body): Json<Changes>,
) -> Response {
    if body.changes.is_empty()
        && !body.touches_project_file()
        && body.layout.is_none()
        && body.documentation.is_none()
    {
        return error(
            StatusCode::BAD_REQUEST,
            "nothing to save: there are no changes",
        );
    }
    let loaded = match load_with(&state, &id, None, &body.changes, body.layout.as_ref()) {
        Ok(l) => l,
        Err(problem) => return problem.into_response(),
    };
    let mut project = loaded.project.clone();
    body.apply_to(&mut project);
    let project_text = project.to_text();
    let mut files = vec![(loaded.project.model.as_str(), Some(loaded.text.as_str()))];
    if body.touches_project_file() {
        files.push((PROJECT_FILE, Some(project_text.as_str())));
    }
    let layout_file = loaded.project.layout_file();
    let layout_text = body
        .layout
        .as_ref()
        .map(|tree| layout_text(tree, &loaded.project.model));
    if let Some(text) = &layout_text {
        files.push((layout_file.as_str(), Some(text.as_str())));
    }
    let documentation = body
        .documentation
        .as_deref()
        .map(|text| text.trim_end().to_string() + "\n");
    if let Some(text) = &documentation {
        files.push((
            DOCUMENTATION_FILE,
            (text.trim() != "").then_some(text.as_str()),
        ));
    }
    match state.projects.save(&id, &files, &body.message) {
        Ok(commit) => {
            let problems = exposed(&state, &id, None).map_or_else(|e| vec![e], |(_, p)| p);
            Json(json!({ "ok": true, "saved": commit, "decision_problems": problems }))
                .into_response()
        }
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
    match exposed(&state, &id, None) {
        Err(e) => return not_found_or_bad(e),
        Ok((_, problems)) if !problems.is_empty() => {
            return invalid(project_problems(&problems));
        }
        Ok(_) => {}
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
    /// A whole new layout, for unsaved edits that add or move objects.
    layout: Option<Value>,
    /// Overrides the model's number of replications.
    replications: Option<u32>,
    /// A name for the run; defaults to the project and version.
    label: Option<String>,
}

async fn start_run(
    State(state): State<Arc<AppState>>,
    axum::Extension(role): axum::Extension<Role>,
    Json(request): Json<RunRequest>,
) -> Response {
    if role == Role::Decider
        && (request.version.is_none() || !request.changes.is_empty() || request.layout.is_some())
    {
        return error(
            StatusCode::FORBIDDEN,
            "people who decide can run published versions as they are; running the working copy or changes needs a modeler",
        );
    }
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
    let loaded = match load_with(
        &state,
        &request.project,
        version.as_deref(),
        &request.changes,
        request.layout.as_ref(),
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
    match enqueue(
        &state,
        &request.project,
        version_name,
        label,
        loaded,
        request.replications,
    ) {
        Ok(id) => {
            let jobs = state.jobs.lock().expect("jobs lock");
            let job = jobs.iter().find(|j| j.id == id).expect("just queued");
            (StatusCode::ACCEPTED, Json(job.to_json())).into_response()
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

/// Queues a run and returns its id.
fn enqueue(
    state: &AppState,
    project: &str,
    version: String,
    label: String,
    loaded: Loaded,
    replications: Option<u32>,
) -> Result<String, String> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let id = format!("{secs}-{}", state.next_id.fetch_add(1, Ordering::Relaxed));
    let job = Job {
        id: id.clone(),
        label,
        project: project.to_string(),
        version,
        total: replications.unwrap_or(loaded.model.spec.experiment.replications),
        model: loaded.model,
        layout: loaded.layout,
        model_text: loaded.text,
        replications,
        progress: Arc::new(AtomicU32::new(0)),
        status: Status::Queued,
        error: None,
        files: Vec::new(),
        queued_at: secs,
        measures: None,
    };
    state.jobs.lock().expect("jobs lock").push(job);
    state
        .queue
        .send(id.clone())
        .map_err(|_| "the run queue has stopped".to_string())?;
    Ok(id)
}

/// A queued, running or finished run of exactly this model, to reuse.
fn same_run(
    state: &AppState,
    project: &str,
    version: &str,
    loaded: &Loaded,
    replications: Option<u32>,
) -> Option<String> {
    let total = replications.unwrap_or(loaded.model.spec.experiment.replications);
    state
        .jobs
        .lock()
        .expect("jobs lock")
        .iter()
        .rev()
        .find(|j| {
            j.project == project
                && j.version == version
                && j.total == total
                && j.status != Status::Failed
                && j.model_text == loaded.text
        })
        .map(|j| j.id.clone())
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

/// Removes a finished run and its result folder. Comparisons using it then say the run is gone.
async fn delete_run(State(state): State<Arc<AppState>>, UrlPath(id): UrlPath<String>) -> Response {
    {
        let mut jobs = state.jobs.lock().expect("jobs lock");
        let Some(i) = jobs.iter().position(|j| j.id == id) else {
            return error(StatusCode::NOT_FOUND, format!("no run `{id}`"));
        };
        if matches!(jobs[i].status, Status::Queued | Status::Running) {
            return error(
                StatusCode::CONFLICT,
                format!("run `{id}` hasn't finished; delete it when it's done"),
            );
        }
        jobs.remove(i);
    }
    match fs::remove_dir_all(state.runs.join(&id)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("can't remove the results of run `{id}`: {e}"),
        ),
        _ => StatusCode::NO_CONTENT.into_response(),
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

// --- managers' scenarios -------------------------------------------------------------

/// A scenario as the API shows it: with what it changes, in words.
fn scenario_json(scenario: &crate::scenarios::Scenario, exposed: Option<&Exposed>) -> Value {
    let mut value = json!(scenario);
    value["summary"] = json!(exposed.map(|e| e.describe(&scenario.decisions)));
    value
}

async fn list_scenarios(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Query(query): Query<VersionQuery>,
) -> Response {
    if let Err(e) = state.projects.dir(&id) {
        return not_found_or_bad(e);
    }
    let version = match query.version.as_deref() {
        None => None,
        Some(v) => match state.projects.version_tag(&id, v) {
            Ok(tag) => Some(tag),
            Err(e) => return not_found_or_bad(e),
        },
    };
    let scenarios = match state.scenarios.list(&id) {
        Ok(s) => s,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let mut described = std::collections::HashMap::new();
    let list: Vec<Value> = scenarios
        .iter()
        .filter(|s| version.as_ref().is_none_or(|v| &s.version == v))
        .map(|s| {
            let exposed = described
                .entry(s.version.clone())
                .or_insert_with(|| exposed(&state, &id, Some(&s.version)).ok().map(|(e, _)| e));
            scenario_json(s, exposed.as_ref())
        })
        .collect();
    Json(json!({ "scenarios": list })).into_response()
}

/// Checks a manager's picks against a published version: the changes they
/// make, or why they can't be used.
fn check_picks(
    state: &AppState,
    id: &str,
    version: &str,
    picks: &Map<String, Value>,
) -> Result<(Exposed, Map<String, Value>), LoadProblem> {
    let (exposed, _) = exposed(state, id, Some(version)).map_err(LoadProblem::Request)?;
    let changes = exposed.changes_for(picks).map_err(|message| {
        LoadProblem::Invalid(vec![
            json!({ "change": null, "file": null, "line": null, "column": null, "message": message }),
        ])
    })?;
    load(state, id, Some(version), &changes)?;
    Ok((exposed, changes))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewScenario {
    name: Option<String>,
    /// `latest` (the default) or `vN`; stored as the version's name.
    version: Option<String>,
    #[serde(default)]
    decisions: Map<String, Value>,
}

async fn create_scenario(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Json(body): Json<NewScenario>,
) -> Response {
    let version = match state
        .projects
        .version_tag(&id, body.version.as_deref().unwrap_or("latest"))
    {
        Ok(tag) => tag,
        Err(e) => return not_found_or_bad(e),
    };
    let exposed = match check_picks(&state, &id, &version, &body.decisions) {
        Ok((exposed, _)) => exposed,
        Err(problem) => return problem.into_response(),
    };
    let name = match body.name.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => match exposed.describe(&body.decisions) {
            summary if summary.is_empty() => format!("As published in {version}"),
            summary => summary,
        },
    };
    match state.scenarios.add(&id, &name, &version, body.decisions) {
        Ok(scenario) => (
            StatusCode::CREATED,
            Json(scenario_json(&scenario, Some(&exposed))),
        )
            .into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioUpdate {
    name: Option<String>,
    decisions: Option<Map<String, Value>>,
}

async fn update_scenario(
    State(state): State<Arc<AppState>>,
    UrlPath((id, scenario)): UrlPath<(String, String)>,
    Json(body): Json<ScenarioUpdate>,
) -> Response {
    let existing = match state.scenarios.get(&id, &scenario) {
        Ok(s) => s,
        Err(e) => return error(StatusCode::NOT_FOUND, e),
    };
    if body.name.as_deref().is_some_and(|n| n.trim().is_empty()) {
        return error(StatusCode::BAD_REQUEST, "a scenario needs a name");
    }
    let picks = body.decisions.as_ref().unwrap_or(&existing.decisions);
    let exposed = match check_picks(&state, &id, &existing.version, picks) {
        Ok((exposed, _)) => exposed,
        Err(problem) => return problem.into_response(),
    };
    match state.scenarios.update(
        &id,
        &scenario,
        body.name.as_deref().map(str::trim),
        body.decisions,
    ) {
        Ok(updated) => Json(scenario_json(&updated, Some(&exposed))).into_response(),
        Err(e) => error(StatusCode::NOT_FOUND, e),
    }
}

async fn delete_scenario(
    State(state): State<Arc<AppState>>,
    UrlPath((id, scenario)): UrlPath<(String, String)>,
) -> Response {
    match state.scenarios.remove(&id, &scenario) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error(StatusCode::NOT_FOUND, e),
    }
}

// --- comparisons ----------------------------------------------------------------------

/// Scenarios compared with the current setup of one published version. Each
/// is an ordinary run; replication r of every run uses the same random
/// numbers, so the differences are measured replication by replication.
#[derive(Clone)]
struct ComparisonJob {
    id: String,
    project: String,
    version: String,
    exposed: Exposed,
    /// The current setup first, then the scenarios.
    items: Vec<Item>,
}

/// What a comparison's file keeps; what the version exposes is read again.
#[derive(Serialize, Deserialize)]
struct StoredComparison {
    id: String,
    project: String,
    version: String,
    items: Vec<Item>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Item {
    name: String,
    /// The saved scenario's id, if it is one.
    scenario: Option<String>,
    decisions: Map<String, Value>,
    run: String,
}

impl ComparisonJob {
    fn to_json(&self, jobs: &[Job]) -> Value {
        let runs: Vec<Option<&Job>> = self
            .items
            .iter()
            .map(|i| jobs.iter().find(|j| j.id == i.run))
            .collect();
        let mut seen = Vec::new();
        let (mut done, mut total) = (0, 0);
        for job in runs.iter().flatten() {
            if !seen.contains(&&job.id) {
                seen.push(&job.id);
                done += job.progress.load(Ordering::Relaxed);
                total += job.total;
            }
        }
        let failed = self
            .items
            .iter()
            .zip(&runs)
            .find(|(_, j)| j.is_none_or(|j| j.status == Status::Failed));
        let status = if failed.is_some() {
            Status::Failed
        } else if runs.iter().flatten().all(|j| j.status == Status::Done) {
            Status::Done
        } else if runs.iter().flatten().all(|j| j.status == Status::Queued) {
            Status::Queued
        } else {
            Status::Running
        };
        let error = failed.map(|(item, job)| {
            format!(
                "{}: {}",
                item.name,
                job.and_then(|j| j.error.clone())
                    .unwrap_or_else(|| "the run is gone".into())
            )
        });
        let base = runs[0].and_then(|j| j.measures.clone());
        let scenarios: Vec<Value> = self
            .items
            .iter()
            .zip(&runs)
            .skip(1)
            .map(|(item, job)| {
                let results = match (&base, job.and_then(|j| j.measures.clone())) {
                    (Some(base), Some(measures)) if status == Status::Done => {
                        json!(
                            self.exposed
                                .results
                                .iter()
                                .filter_map(|spec| {
                                    let find = |list: &[Measure]| {
                                        list.iter()
                                            .find(|m| {
                                                m.object == spec.object && m.name == spec.measure
                                            })
                                            .cloned()
                                    };
                                    let (b, s) = (find(base)?, find(&measures)?);
                                    Some(plain::describe(spec, &compare_pair(b, &s)))
                                })
                                .collect::<Vec<_>>()
                        )
                    }
                    _ => Value::Null,
                };
                json!({
                    "name": item.name,
                    "scenario": item.scenario,
                    "decisions": item.decisions,
                    "summary": self.exposed.describe(&item.decisions),
                    "run": item.run,
                    "results": results,
                })
            })
            .collect();
        json!({
            "id": self.id,
            "project": self.project,
            "version": self.version,
            "status": status,
            "progress": { "done": done, "total": total },
            "error": error,
            "replications": runs[0].map(|j| j.total),
            "current": { "name": self.items[0].name, "run": self.items[0].run },
            "results": self.exposed.results,
            "scenarios": scenarios,
        })
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ScenarioRef {
    /// A saved scenario's id.
    Saved(String),
    /// Picks given in the request.
    Inline {
        name: Option<String>,
        #[serde(default)]
        decisions: Map<String, Value>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompareRequest {
    project: String,
    /// `latest` or `vN`. Defaults to the saved scenarios' version, or `latest`.
    version: Option<String>,
    scenarios: Vec<ScenarioRef>,
    /// Overrides the model's number of replications.
    replications: Option<u32>,
}

const MAX_SCENARIOS: usize = 12;

async fn start_comparison(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CompareRequest>,
) -> Response {
    let project = request.project.as_str();
    if request.scenarios.is_empty() || request.scenarios.len() > MAX_SCENARIOS {
        return error(
            StatusCode::BAD_REQUEST,
            format!("compare 1 to {MAX_SCENARIOS} scenarios with the current setup"),
        );
    }
    if request.replications == Some(0) {
        return error(StatusCode::BAD_REQUEST, "`replications` must be at least 1");
    }
    // The saved scenarios, and the version they were all made on.
    let mut saved = Vec::new();
    for r in &request.scenarios {
        if let ScenarioRef::Saved(id) = r {
            match state.scenarios.get(project, id) {
                Ok(s) => saved.push(s),
                Err(e) => return error(StatusCode::NOT_FOUND, e),
            }
        }
    }
    let requested = match request.version.as_deref() {
        Some(v) => match state.projects.version_tag(project, v) {
            Ok(tag) => Some(tag),
            Err(e) => return not_found_or_bad(e),
        },
        None => None,
    };
    let mut versions: Vec<&str> = saved.iter().map(|s| s.version.as_str()).collect();
    versions.extend(requested.as_deref());
    versions.sort_unstable();
    versions.dedup();
    let version = match versions.as_slice() {
        [] => match state.projects.version_tag(project, "latest") {
            Ok(tag) => tag,
            Err(e) => return not_found_or_bad(e),
        },
        [one] => one.to_string(),
        several => {
            return error(
                StatusCode::BAD_REQUEST,
                format!(
                    "these scenarios were made on different versions ({}); compare scenarios of one version at a time",
                    several.join(", ")
                ),
            );
        }
    };

    let mut wanted = vec![("Current setup".to_string(), None, Map::new())];
    let mut saved = saved.into_iter();
    for r in request.scenarios {
        wanted.push(match r {
            ScenarioRef::Saved(_) => {
                let s = saved.next().expect("one per saved reference");
                (s.name, Some(s.id), s.decisions)
            }
            ScenarioRef::Inline { name, decisions } => (
                name.unwrap_or_else(|| format!("Scenario {}", wanted.len())),
                None,
                decisions,
            ),
        });
    }
    let mut exposed_found = None;
    let mut items = Vec::new();
    for (name, scenario, picks) in wanted {
        let (exposed, changes) = match check_picks(&state, project, &version, &picks) {
            Ok(found) => found,
            Err(problem) => return problem.into_response(),
        };
        let loaded = match load(&state, project, Some(&version), &changes) {
            Ok(l) => l,
            Err(problem) => return problem.into_response(),
        };
        let run = match same_run(&state, project, &version, &loaded, request.replications) {
            Some(id) => id,
            None => {
                let label = format!("{} {version} · {name}", loaded.project.name);
                match enqueue(
                    &state,
                    project,
                    version.clone(),
                    label,
                    loaded,
                    request.replications,
                ) {
                    Ok(id) => id,
                    Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
                }
            }
        };
        exposed_found = Some(exposed);
        items.push(Item {
            name,
            scenario,
            decisions: picks,
            run,
        });
    }
    let comparison = ComparisonJob {
        id: format!("c{}", state.next_id.fetch_add(1, Ordering::Relaxed)),
        project: project.to_string(),
        version,
        exposed: exposed_found.unwrap_or_default(),
        items,
    };
    let stored = StoredComparison {
        id: comparison.id.clone(),
        project: comparison.project.clone(),
        version: comparison.version.clone(),
        items: comparison.items.clone(),
    };
    let dir = state.runs.join(COMPARISONS_DIR);
    if let Err(e) = fs::create_dir_all(&dir).and_then(|()| {
        fs::write(
            dir.join(format!("{}.json", stored.id)),
            serde_json::to_string_pretty(&stored).expect("a comparison is plain data"),
        )
    }) {
        eprintln!(
            "comparison {} won't be kept after a restart: {e}",
            stored.id
        );
    }
    let body = comparison.to_json(&state.jobs.lock().expect("jobs lock"));
    state
        .comparisons
        .lock()
        .expect("comparisons lock")
        .push(comparison);
    (StatusCode::ACCEPTED, Json(body)).into_response()
}

async fn list_comparisons(State(state): State<Arc<AppState>>) -> Response {
    let comparisons = state.comparisons.lock().expect("comparisons lock").clone();
    let jobs = state.jobs.lock().expect("jobs lock");
    let list: Vec<Value> = comparisons.iter().rev().map(|c| c.to_json(&jobs)).collect();
    Json(json!({ "comparisons": list })).into_response()
}

async fn get_comparison(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
) -> Response {
    let comparison = state
        .comparisons
        .lock()
        .expect("comparisons lock")
        .iter()
        .find(|c| c.id == id)
        .cloned();
    match comparison {
        Some(c) => Json(c.to_json(&state.jobs.lock().expect("jobs lock"))).into_response(),
        None => error(StatusCode::NOT_FOUND, format!("no comparison `{id}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// A server over a fresh data folder with a project from each template.
    fn app(name: &str) -> (Router, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("simulancer-serve-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("projects")).unwrap();
        let projects = Projects::new(root.join("projects"));
        for t in projects::TEMPLATES {
            projects::create_from_template(&projects, t.id, t.id, None).unwrap();
        }
        (reopen(&root), root)
    }

    /// A server started again over an existing data folder.
    fn reopen(root: &std::path::Path) -> Router {
        let state = AppState::start(
            Projects::new(root.join("projects")),
            Scenarios::new(root.join("scenarios")),
            root.join("runs"),
            2,
        );
        router(state)
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
        assert_eq!(projects.len(), 6);
        assert_eq!(projects[4]["id"], "two-cells");
        assert_eq!(projects[4]["name"], "Two cells");
        assert_eq!(projects[4]["published"]["name"], "v1");
        fs::remove_dir_all(root).unwrap();
    }

    /// Every template loads, with a layout, and its decisions and
    /// results can be published; and projects can start from it.
    #[tokio::test]
    async fn templates_are_valid_and_projects_start_from_them() {
        let (app, root) = app("templates");
        let (_, body) = call(&app, "GET", "/api/v1/templates", None).await;
        let templates = body["templates"].as_array().unwrap();
        assert_eq!(templates.len(), projects::TEMPLATES.len());
        assert_eq!(templates[1]["name"], "Bank branch");
        assert!(
            templates[1]["tags"]
                .as_array()
                .unwrap()
                .contains(&json!("services"))
        );
        for template in projects::TEMPLATES {
            let (_, project) = call(
                &app,
                "GET",
                &format!("/api/v1/projects/{}", template.id),
                None,
            )
            .await;
            assert_eq!(
                project["valid"], true,
                "{}: {}",
                template.id, project["errors"]
            );
            assert!(
                project["layout"].is_object(),
                "{} has a layout",
                template.id
            );
            assert_eq!(project["decision_problems"], json!([]), "{}", template.id);
            assert!(
                !project["decisions"].as_array().unwrap().is_empty(),
                "{}",
                template.id
            );
            assert!(
                !project["results"].as_array().unwrap().is_empty(),
                "{}",
                template.id
            );
        }

        let (status, _) = call(
            &app,
            "POST",
            "/api/v1/projects",
            Some(json!({ "id": "my-bank", "name": "My bank", "template": "bank-branch" })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (_, project) = call(&app, "GET", "/api/v1/projects/my-bank?version=latest", None).await;
        assert_eq!(
            (project["name"].as_str(), project["version"].as_str()),
            (Some("My bank"), Some("v1"))
        );
        for (body, expected) in [
            (
                json!({ "id": "x", "template": "nope" }),
                StatusCode::NOT_FOUND,
            ),
            (
                json!({ "id": "x", "template": "warehouse", "from": "two-cells" }),
                StatusCode::BAD_REQUEST,
            ),
            (
                json!({ "id": "my-bank", "template": "warehouse" }),
                StatusCode::CONFLICT,
            ),
        ] {
            let (status, _) = call(&app, "POST", "/api/v1/projects", Some(body)).await;
            assert_eq!(status, expected);
        }
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
        assert_eq!(project["layout"]["format"], "simulancer-layout/0.1");
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
    async fn deletes_finished_runs_and_unfinished_ones_on_start() {
        let (app, root) = app("delete-runs");
        let body = json!({ "project": "two-cells", "replications": 2 });
        let (_, run) = call(&app, "POST", "/api/v1/runs", Some(body)).await;
        let id = run["id"].as_str().unwrap().to_string();
        let (status, _) = call(&app, "DELETE", &format!("/api/v1/runs/{id}"), None).await;
        assert!(
            [StatusCode::CONFLICT, StatusCode::NO_CONTENT].contains(&status),
            "a queued or running run can't be deleted: {status}"
        );
        wait_for(&app, &id).await;
        assert!(root.join("runs").join(&id).join("events.jsonl").exists());
        let (status, _) = call(&app, "DELETE", &format!("/api/v1/runs/{id}"), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(!root.join("runs").join(&id).exists());
        let (_, list) = call(&app, "GET", "/api/v1/runs", None).await;
        assert_eq!(list["runs"], json!([]));
        let (status, _) = call(&app, "DELETE", &format!("/api/v1/runs/{id}"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // A run that never finished (no run.json) is removed on the next start;
        // anything not named like a run is left alone.
        let runs = root.join("runs");
        fs::create_dir_all(runs.join("1791642661-3")).unwrap();
        fs::create_dir_all(runs.join("notes")).unwrap();
        fs::write(runs.join("2-2"), "a file").unwrap();
        let app = reopen(&root);
        assert!(!runs.join("1791642661-3").exists());
        assert!(runs.join("notes").exists() && runs.join("2-2").exists());
        let (_, list) = call(&app, "GET", "/api/v1/runs", None).await;
        assert_eq!(list["runs"], json!([]));
        fs::remove_dir_all(root).unwrap();
    }

    /// Finished runs and comparisons come back after a restart, identical, and
    /// new ids don't collide with them.
    #[tokio::test]
    async fn runs_and_comparisons_survive_a_restart() {
        let (app, root) = app("restart");
        let request = json!({
            "project": "two-cells",
            "scenarios": [{ "name": "Two inspectors", "decisions": { "inspectors": 2 } }],
            "replications": 3,
        });
        let (status, started) =
            call(&app, "POST", "/api/v1/comparisons", Some(request.clone())).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{started}");
        let id = started["id"].as_str().unwrap().to_string();
        for run in [&started["current"]["run"], &started["scenarios"][0]["run"]] {
            wait_for(&app, run.as_str().unwrap()).await;
        }
        let (_, comparison) = call(&app, "GET", &format!("/api/v1/comparisons/{id}"), None).await;
        assert_eq!(comparison["status"], "done", "{comparison}");
        let (_, runs) = call(&app, "GET", "/api/v1/runs", None).await;
        assert_eq!(runs["runs"].as_array().unwrap().len(), 2);

        let app = reopen(&root);
        let (_, after) = call(&app, "GET", "/api/v1/runs", None).await;
        assert_eq!(after, runs);
        let (_, again) = call(&app, "GET", &format!("/api/v1/comparisons/{id}"), None).await;
        assert_eq!(again, comparison);
        let (_, list) = call(&app, "GET", "/api/v1/comparisons", None).await;
        assert_eq!(list["comparisons"][0]["id"], json!(id));
        let first = &after["runs"][1];
        let (status, _) = call(
            &app,
            "GET",
            &format!(
                "/api/v1/runs/{}/files/report.html",
                first["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // The same comparison reuses the kept runs; a new one gets a new id.
        let (_, reused) = call(&app, "POST", "/api/v1/comparisons", Some(request)).await;
        assert_ne!(reused["id"], json!(id));
        assert_eq!(reused["current"]["run"], started["current"]["run"]);
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
        assert!(run["queued_at"].as_u64().unwrap() > 1_700_000_000, "{run}");
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
    async fn renames_archives_restores_and_deletes_projects() {
        let (app, root) = app("manage");
        let ids = |v: &Value| -> Vec<String> {
            v["projects"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["id"].as_str().unwrap().to_string())
                .collect()
        };
        let (status, body) = call(
            &app,
            "PATCH",
            "/api/v1/projects/two-cells",
            Some(json!({ "name": "Two mills", "description": "" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            (body["name"].as_str(), body["saved"].as_bool()),
            (Some("Two mills"), Some(true))
        );
        assert_eq!(body["description"], Value::Null);
        // A published version shows the project's current name.
        let (_, v1) = call(&app, "GET", "/api/v1/projects/two-cells?version=v1", None).await;
        assert_eq!(v1["name"], "Two mills");

        // Delete needs the archive first.
        let (status, _) = call(&app, "DELETE", "/api/v1/projects/two-cells", None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        call(
            &app,
            "POST",
            "/api/v1/projects/two-cells/scenarios",
            Some(json!({ "version": "v1", "name": "More inspectors", "decisions": { "inspectors": 2 } })),
        )
        .await;
        let (status, _) = call(&app, "POST", "/api/v1/projects/two-cells/archive", None).await;
        assert_eq!(status, StatusCode::OK);
        let (_, list) = call(&app, "GET", "/api/v1/projects", None).await;
        assert!(!ids(&list).contains(&"two-cells".to_string()));
        let (_, archived) = call(&app, "GET", "/api/v1/projects?archived=true", None).await;
        assert_eq!(ids(&archived), ["two-cells"]);
        assert_eq!(archived["projects"][0]["name"], "Two mills");
        let (status, _) = call(&app, "GET", "/api/v1/projects/two-cells", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, _) = call(&app, "POST", "/api/v1/projects/two-cells/restore", None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(&app, "POST", "/api/v1/projects/two-cells/restore", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (_, scenarios) = call(
            &app,
            "GET",
            "/api/v1/projects/two-cells/scenarios?version=v1",
            None,
        )
        .await;
        assert_eq!(
            scenarios["scenarios"].as_array().unwrap().len(),
            1,
            "scenarios survive the archive"
        );

        call(&app, "POST", "/api/v1/projects/two-cells/archive", None).await;
        let (status, _) = call(&app, "DELETE", "/api/v1/projects/two-cells", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, archived) = call(&app, "GET", "/api/v1/projects?archived=true", None).await;
        assert!(ids(&archived).is_empty());
        assert!(!root.join("scenarios/two-cells.json").exists());
        let (status, _) = call(&app, "DELETE", "/api/v1/projects/two-cells", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        fs::remove_dir_all(root).unwrap();
    }

    /// Deleting a project takes its runs and comparisons along, so a new
    /// project with the same id starts clean, before and after a restart.
    #[tokio::test]
    async fn deleting_a_project_removes_its_runs_and_comparisons() {
        let (app, root) = app("forget");
        let compare = |project: &str| {
            json!({
                "project": project,
                "scenarios": [{ "name": "Two inspectors", "decisions": { "inspectors": 2 } }],
                "replications": 2,
            })
        };
        let mut runs = Vec::new();
        for _ in 0..2 {
            let (status, c) = call(
                &app,
                "POST",
                "/api/v1/comparisons",
                Some(compare("two-cells")),
            )
            .await;
            assert_eq!(status, StatusCode::ACCEPTED, "{c}");
            for run in [&c["current"]["run"], &c["scenarios"][0]["run"]] {
                runs.push(run.as_str().unwrap().to_string());
            }
        }
        // Another project's run, which has to stay.
        let other = json!({ "project": "warehouse", "replications": 2 });
        let (status, run) = call(&app, "POST", "/api/v1/runs", Some(other)).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{run}");
        runs.push(run["id"].as_str().unwrap().to_string());
        for run in &runs {
            wait_for(&app, run).await;
        }
        let comparisons_dir = root.join("runs").join(COMPARISONS_DIR);
        assert_eq!(fs::read_dir(&comparisons_dir).unwrap().count(), 2);

        call(&app, "POST", "/api/v1/projects/two-cells/archive", None).await;
        let (status, _) = call(&app, "DELETE", "/api/v1/projects/two-cells", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, list) = call(&app, "GET", "/api/v1/comparisons", None).await;
        assert_eq!(list["comparisons"], json!([]));
        let (_, left) = call(&app, "GET", "/api/v1/runs", None).await;
        let left: Vec<&str> = left["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["project"].as_str().unwrap())
            .collect();
        assert_eq!(left, ["warehouse"]);
        assert_eq!(fs::read_dir(&comparisons_dir).unwrap().count(), 0);
        let folders = fs::read_dir(root.join("runs"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name() != COMPARISONS_DIR)
            .count();
        assert_eq!(folders, 1, "only warehouse's run folder is left");

        // A new two-cells has nothing of the old one, also after a restart.
        let body = json!({ "id": "two-cells", "template": "two-cells" });
        let (status, _) = call(&app, "POST", "/api/v1/projects", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED);
        let app = reopen(&root);
        let (_, list) = call(&app, "GET", "/api/v1/comparisons", None).await;
        assert_eq!(list["comparisons"], json!([]));
        let (_, all) = call(&app, "GET", "/api/v1/runs", None).await;
        assert_eq!(all["runs"].as_array().unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    /// Results left by a project deleted by hand (or before deleting took them
    /// along) go when a project with that id is created.
    #[tokio::test]
    async fn a_new_project_clears_what_an_old_one_left() {
        let (app, root) = app("leftovers");
        let request = json!({
            "project": "two-cells",
            "scenarios": [{ "name": "Two inspectors", "decisions": { "inspectors": 2 } }],
            "replications": 2,
        });
        let (_, c) = call(&app, "POST", "/api/v1/comparisons", Some(request)).await;
        for run in [&c["current"]["run"], &c["scenarios"][0]["run"]] {
            wait_for(&app, run.as_str().unwrap()).await;
        }
        fs::remove_dir_all(root.join("projects/two-cells")).unwrap();
        let app = reopen(&root);
        let comparisons_dir = root.join("runs").join(COMPARISONS_DIR);
        assert_eq!(fs::read_dir(&comparisons_dir).unwrap().count(), 1);

        let body = json!({ "id": "two-cells", "template": "two-cells" });
        let (status, _) = call(&app, "POST", "/api/v1/projects", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(fs::read_dir(&comparisons_dir).unwrap().count(), 0);
        let (_, runs) = call(&app, "GET", "/api/v1/runs", None).await;
        assert_eq!(runs["runs"], json!([]));
        let app = reopen(&root);
        let (_, list) = call(&app, "GET", "/api/v1/comparisons", None).await;
        assert_eq!(list["comparisons"], json!([]));
        fs::remove_dir_all(root).unwrap();
    }

    /// A project's runs check the precision of the results it shows on the
    /// Simulation page, not the sinks'.
    #[tokio::test]
    async fn runs_check_the_projects_results() {
        let (app, root) = app("precision");
        let (status, run) = call(
            &app,
            "POST",
            "/api/v1/runs",
            Some(json!({ "project": "two-cells", "version": "latest", "replications": 4 })),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{run}");
        let id = run["id"].as_str().unwrap();
        wait_for(&app, id).await;
        let (_, summary) = call(
            &app,
            "GET",
            &format!("/api/v1/runs/{id}/files/summary.json"),
            None,
        )
        .await;
        let labels: Vec<&str> = summary["precision_check"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["label"].as_str().unwrap())
            .collect();
        assert_eq!(
            labels,
            ["Parts shipped per hour", "Lead time", "Inspection busy"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// With roles on, a decider builds and compares scenarios of published
    /// versions, and gets a 403 on everything that changes a model or project.
    #[tokio::test]
    async fn deciders_cant_change_models() {
        let root =
            std::env::temp_dir().join(format!("simulancer-serve-{}-roles", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("projects")).unwrap();
        let projects = Projects::new(root.join("projects"));
        projects::create_from_template(&projects, "two-cells", "two-cells", None).unwrap();
        let state = AppState::start(
            projects,
            Scenarios::new(root.join("scenarios")),
            root.join("runs"),
            2,
        )
        .with_roles(Some(Roles {
            header: "x-forwarded-groups".parse().unwrap(),
            modelers: vec!["engineering".into()],
        }));
        let app = router(state);
        let as_who = |groups: &'static str| {
            let app = app.clone();
            move |method: &'static str, uri: String, body: Option<Value>| {
                let app = app.clone();
                async move {
                    let request = Request::builder()
                        .method(method)
                        .uri(uri)
                        .header(header::CONTENT_TYPE, "application/json")
                        .header("x-forwarded-groups", groups)
                        .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
                        .unwrap();
                    let response = app.oneshot(request).await.unwrap();
                    let status = response.status();
                    let bytes = response.into_body().collect().await.unwrap().to_bytes();
                    (
                        status,
                        serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
                    )
                }
            }
        };
        let decider = as_who("managers");
        let modeler = as_who("staff, engineering");

        let (_, me) = decider("GET", "/api/v1/me".into(), None).await;
        assert_eq!(me, json!({ "role": "decider", "roles": true }));
        let (_, me) = modeler("GET", "/api/v1/me".into(), None).await;
        assert_eq!(me["role"], "modeler");

        // What a decider may do.
        let (status, _) = decider("GET", "/api/v1/projects/two-cells".into(), None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, scenario) = decider(
            "POST",
            "/api/v1/projects/two-cells/scenarios".into(),
            Some(json!({ "version": "v1", "name": "Two inspectors", "decisions": { "inspectors": 2 } })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{scenario}");
        let (status, c) = decider(
            "POST",
            "/api/v1/comparisons".into(),
            Some(
                json!({ "project": "two-cells", "scenarios": [scenario["id"]], "replications": 2 }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{c}");
        let (status, _) = decider(
            "POST",
            "/api/v1/runs".into(),
            Some(json!({ "project": "two-cells", "version": "v1", "replications": 2 })),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        // What only modelers may do.
        for (method, uri, body) in [
            ("GET", "/modeling".to_string(), None),
            (
                "POST",
                "/api/v1/projects".to_string(),
                Some(json!({ "id": "x" })),
            ),
            (
                "PATCH",
                "/api/v1/projects/two-cells".to_string(),
                Some(json!({ "name": "Y" })),
            ),
            (
                "POST",
                "/api/v1/projects/two-cells/save".to_string(),
                Some(json!({ "message": "m" })),
            ),
            (
                "POST",
                "/api/v1/projects/two-cells/publish".to_string(),
                Some(json!({})),
            ),
            (
                "POST",
                "/api/v1/projects/two-cells/archive".to_string(),
                None,
            ),
            ("DELETE", "/api/v1/projects/two-cells".to_string(), None),
            (
                "POST",
                "/api/v1/runs".to_string(),
                Some(json!({ "project": "two-cells" })),
            ),
            (
                "POST",
                "/api/v1/runs".to_string(),
                Some(
                    json!({ "project": "two-cells", "version": "v1", "changes": { "experiment.seed": 2 } }),
                ),
            ),
        ] {
            let (status, _) = decider(method, uri.clone(), body).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
        }
        let (status, _) = modeler(
            "PATCH",
            "/api/v1/projects/two-cells".into(),
            Some(json!({ "name": "Two mills" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
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
    async fn documentation_is_saved_with_the_model() {
        let (app, root) = app("documentation");
        let uri = "/api/v1/projects/bank-branch";
        let (_, project) = call(&app, "GET", uri, None).await;
        assert!(
            project["documentation"]
                .as_str()
                .unwrap()
                .starts_with("# Bank branch\n")
        );
        assert!(
            project["documentation_html"]
                .as_str()
                .unwrap()
                .starts_with("<h1>Bank branch</h1>")
        );

        // A copy keeps it; a blank project has none.
        let copy = json!({ "id": "branch-2", "from": "bank-branch" });
        call(&app, "POST", "/api/v1/projects", Some(copy)).await;
        let (_, copied) = call(&app, "GET", "/api/v1/projects/branch-2", None).await;
        assert_eq!(copied["documentation"], project["documentation"]);
        call(
            &app,
            "POST",
            "/api/v1/projects",
            Some(json!({ "id": "empty" })),
        )
        .await;
        let (_, blank) = call(&app, "GET", "/api/v1/projects/empty", None).await;
        assert_eq!(blank["documentation"], "");
        assert_eq!(blank["documentation_html"], "");

        // Checking renders the draft, safely; nothing is written.
        let draft = "# Notes\n\nTellers <script>x()</script> rotate.";
        let (_, checked) = call(
            &app,
            "POST",
            &format!("{uri}/check"),
            Some(json!({ "documentation": draft })),
        )
        .await;
        let html = checked["documentation_html"].as_str().unwrap();
        assert!(
            html.contains("<h1>Notes</h1>") && !html.contains("<script"),
            "{html}"
        );
        assert!(
            fs::read_to_string(root.join("projects/bank-branch/README.md"))
                .unwrap()
                .starts_with("# Bank")
        );

        // Saving writes README.md as a version; published versions keep theirs.
        let save = json!({ "documentation": draft, "message": "Explained the tellers" });
        let (status, saved) = call(&app, "POST", &format!("{uri}/save"), Some(save)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["saved"]["message"], "Explained the tellers");
        let (_, project) = call(&app, "GET", uri, None).await;
        assert_eq!(project["documentation"], format!("{draft}\n"));
        let (_, published) = call(&app, "GET", &format!("{uri}?version=v1"), None).await;
        assert!(
            published["documentation"]
                .as_str()
                .unwrap()
                .starts_with("# Bank branch")
        );

        // Emptied, the file goes.
        let (status, _) = call(
            &app,
            "POST",
            &format!("{uri}/save"),
            Some(json!({ "documentation": "  \n" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!root.join("projects/bank-branch/README.md").exists());
        let (_, project) = call(&app, "GET", uri, None).await;
        assert_eq!(project["documentation"], "");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn new_projects_start_as_a_simple_line() {
        let (app, root) = app("template");
        let (status, _) = call(
            &app,
            "POST",
            "/api/v1/projects",
            Some(json!({ "id": "line-3", "name": "Line 3" })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (_, project) = call(&app, "GET", "/api/v1/projects/line-3", None).await;
        assert_eq!(project["valid"], true, "{project}");
        assert_eq!(project["tree"]["name"], "line-3");
        assert!(project["layout"]["objects"]["machine"].is_object());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn topology_edits_with_a_new_layout() {
        let (app, root) = app("topology");
        let (_, project) = call(&app, "GET", "/api/v1/projects/conveyor-line", None).await;
        let mut layout = project["layout"].clone();
        // Remove the mill: its conveyors go, and the lathe ships directly.
        let changes = json!({
            "servers.mill": null,
            "conveyors.c3": null,
            "conveyors.c2.to": "shipping",
        });
        layout["objects"].as_object_mut().unwrap().remove("mill");
        layout["conveyors"].as_object_mut().unwrap().remove("c3");
        layout["conveyors"]["c2"]["path"] = json!([[14.5, 0, 0], [22.5, 0, 0], [22.5, 0, 0.5]]);
        let check = json!({ "changes": changes, "layout": layout });
        let (_, checked) = call(
            &app,
            "POST",
            "/api/v1/projects/conveyor-line/check",
            Some(check.clone()),
        )
        .await;
        // The mill decision now points at nothing.
        assert_eq!(checked["ok"], true, "{checked}");
        assert!(
            checked["decision_problems"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p.as_str().unwrap().contains("decision `mill`")),
            "{checked}"
        );
        // Without the new layout, the saved one no longer fits.
        let (_, checked) = call(
            &app,
            "POST",
            "/api/v1/projects/conveyor-line/check",
            Some(json!({ "changes": changes })),
        )
        .await;
        assert_eq!(checked["ok"], false);
        assert_eq!(checked["errors"][0]["file"], "model.layout.yaml");

        let mut save = check;
        save["message"] = json!("No mill");
        let (status, saved) = call(
            &app,
            "POST",
            "/api/v1/projects/conveyor-line/save",
            Some(save),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        let (_, project) = call(&app, "GET", "/api/v1/projects/conveyor-line", None).await;
        assert!(project["tree"]["servers"].get("mill").is_none());
        assert!(project["layout"]["objects"].get("mill").is_none());
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
    async fn examples_expose_decisions_and_results() {
        let (app, root) = app("exposed");
        for id in ["two-cells", "conveyor-line", "bike-assembly"] {
            let (_, project) = call(
                &app,
                "GET",
                &format!("/api/v1/projects/{id}?version=latest"),
                None,
            )
            .await;
            assert_eq!(project["decision_problems"], json!([]), "{id}");
            assert!(project["decisions"].as_array().unwrap().len() >= 3, "{id}");
            assert!(project["results"].as_array().unwrap().len() >= 3, "{id}");
        }
        let (_, project) = call(&app, "GET", "/api/v1/projects/two-cells", None).await;
        let inspectors = &project["decisions"][1];
        assert_eq!(inspectors["kind"], "number");
        assert_eq!(inspectors["current"], 1.0);
        assert_eq!(project["results"][1]["unit"], "s");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn publishing_needs_working_decisions() {
        let (app, root) = app("broken-decisions");
        // The engineer removes the inspection station's capacity line, which
        // the `inspectors` decision needs.
        let dir = root.join("projects/two-cells");
        let model = fs::read_to_string(dir.join("model.yaml")).unwrap();
        fs::write(
            dir.join("model.yaml"),
            model.replace("    capacity: 1 ", "    #capacity: 1 "),
        )
        .unwrap();
        let (_, project) = call(&app, "GET", "/api/v1/projects/two-cells", None).await;
        assert!(
            project["decision_problems"][0]
                .as_str()
                .unwrap()
                .contains("has no `servers.inspect.capacity`"),
            "{project}"
        );
        crate::projects::git(&dir, &["commit", "-qam", "Drop capacity"]).unwrap();
        let (status, body) = call(
            &app,
            "POST",
            "/api/v1/projects/two-cells/publish",
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["errors"][0]["file"], "project.yaml");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn engineers_edit_decisions_with_live_checks() {
        let (app, root) = app("edit-decisions");
        let (_, project) = call(&app, "GET", "/api/v1/projects/conveyor-line", None).await;
        let mut decisions = project["project_file"]["decisions"].clone();
        assert!(decisions["buffer"].is_object());
        assert!(
            project["measures"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["object"] == "mill" && m["measure"] == "busy")
        );

        // A new number decision, checked before saving: c3 isn't written in m.
        decisions["exit_buffer"] = json!({ "label": "Exit buffer", "path": "conveyors.c3.speed", "min": 1, "max": 2, "unit": "m" });
        let url = "/api/v1/projects/conveyor-line/check";
        let (_, checked) = call(&app, "POST", url, Some(json!({ "decisions": decisions }))).await;
        assert_eq!(checked["ok"], true);
        assert!(
            checked["decision_problems"][0]
                .as_str()
                .unwrap()
                .contains("write it in m"),
            "{checked}"
        );
        // Fixed, with pending model changes checked together.
        decisions["exit_buffer"] = json!({ "label": "Exit buffer", "path": "conveyors.c3.length", "min": 4, "max": 12, "unit": "m" });
        let results = json!([{ "label": "Mill busy", "object": "mill", "measure": "busy" }]);
        let body = json!({ "changes": { "conveyors.c3.length": "8 m" }, "decisions": decisions, "results": results });
        let (_, checked) = call(&app, "POST", url, Some(body.clone())).await;
        assert_eq!(checked["decision_problems"], json!([]), "{checked}");
        assert_eq!(checked["decisions"][3]["current"], 8.0);
        assert_eq!(checked["results"][0]["unit"], "fraction");

        let mut save = body;
        save["message"] = json!("Exit buffer for managers");
        let (status, saved) = call(
            &app,
            "POST",
            "/api/v1/projects/conveyor-line/save",
            Some(save),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        let (_, project) = call(&app, "GET", "/api/v1/projects/conveyor-line", None).await;
        assert_eq!(project["decisions"].as_array().unwrap().len(), 4);
        assert_eq!(project["results"][0]["label"], "Mill busy");
        assert_eq!(project["tree"]["conveyors"]["c3"]["length"], "8 m");

        // Only decisions changed: still a version.
        let mut fewer = project["project_file"]["decisions"].clone();
        fewer.as_object_mut().unwrap().remove("mill");
        let (status, _) = call(
            &app,
            "POST",
            "/api/v1/projects/conveyor-line/save",
            Some(json!({ "decisions": fewer, "message": "No mill decision" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, project) = call(&app, "GET", "/api/v1/projects/conveyor-line", None).await;
        assert_eq!(project["decisions"].as_array().unwrap().len(), 3);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn managers_save_scenarios_and_compare_them() {
        let (app, root) = app("compare");
        let url = "/api/v1/projects/two-cells/scenarios";
        let (status, two) = call(
            &app,
            "POST",
            url,
            Some(json!({ "decisions": { "inspectors": 2 } })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{two}");
        assert_eq!(two["name"], "Inspectors: 2", "named after its changes");
        assert_eq!(two["version"], "v1", "pinned to the published version");

        for (bad, message) in [
            (json!({ "inspectors": 9 }), "between 1 and 3"),
            (json!({ "arrival_routing": "random" }), "no option"),
            (json!({ "speed": 1 }), "no decision `speed`"),
        ] {
            let (status, body) = call(&app, "POST", url, Some(json!({ "decisions": bad }))).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert!(
                body["errors"][0]["message"]
                    .as_str()
                    .unwrap()
                    .contains(message),
                "{body}"
            );
        }
        let (_, renamed) = call(
            &app,
            "PUT",
            &format!("{url}/s1"),
            Some(json!({ "name": "Second inspector" })),
        )
        .await;
        assert_eq!(renamed["name"], "Second inspector");
        let (_, list) = call(&app, "GET", url, None).await;
        assert_eq!(list["scenarios"][0]["summary"], "Inspectors: 2");

        // Compare the saved scenario, and one that changes nothing.
        let request = json!({
            "project": "two-cells",
            "scenarios": ["s1", { "name": "Same as now", "decisions": { "inspectors": 1 } }],
            "replications": 4,
        });
        let (status, started) = call(&app, "POST", "/api/v1/comparisons", Some(request)).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{started}");
        let id = started["id"].as_str().unwrap();
        assert_eq!(
            started["scenarios"][1]["run"], started["current"]["run"],
            "a scenario that changes nothing reuses the current setup's run"
        );
        let mut done = Value::Null;
        for _ in 0..600 {
            let (_, c) = call(&app, "GET", &format!("/api/v1/comparisons/{id}"), None).await;
            if c["status"] == "done" || c["status"] == "failed" {
                done = c;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(done["status"], "done", "{done}");
        assert_eq!(done["replications"], 4);
        let results = done["scenarios"][0]["results"].as_array().unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[1]["label"], "Lead time");
        let sentence = results[1]["sentence"].as_str().unwrap();
        assert!(sentence.starts_with("Lead time: "), "{sentence}");
        assert!(sentence.contains("between"), "{sentence}");
        // The same model with the same random numbers: identical results.
        let same = &done["scenarios"][1]["results"][0];
        assert_eq!(same["verdict"], "identical", "{same}");

        let (status, _) = call(&app, "DELETE", &format!("{url}/s1"), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = call(
            &app,
            "POST",
            "/api/v1/comparisons",
            Some(json!({ "project": "two-cells", "scenarios": ["s1"] })),
        )
        .await;
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
            ("/", "<script src=\"index.js\">"),
            ("/index.js", "modeling?project="),
            ("/modeling", "<script src=\"modeling.js\">"),
            ("/simulation", "<script src=\"simulation.js\">"),
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
