//! End-to-end tests of the `simulancer` binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_simulancer");

/// The SVG namespace is the only URL the report may contain.
const SVG_NS: &str = "http://www.w3.org/2000/svg";

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/conveyor-line.yaml")
}

/// A fresh, empty directory for one test's output.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("simulancer-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> Output {
    let output = Command::new(BIN).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "simulancer {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Too few replications for the key results get a warning that says how
/// many would do, and the check is in summary.json.
#[test]
fn run_warns_when_replications_are_too_few() {
    let dir = temp_dir("precision");
    let out = run(&[
        "run",
        example().to_str().unwrap(),
        "--replications",
        "3",
        "--output-dir",
        dir.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("warning: 3 replications aren't enough for ± 5%: "),
        "{stdout}"
    );
    assert!(
        stdout.contains("time in system at shipping is ± "),
        "{stdout}"
    );
    let summary: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("summary.json")).unwrap()).unwrap();
    let check = &summary["precision_check"];
    assert_eq!(check["target"], 0.05);
    let time = check["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["measure"] == "time in system")
        .unwrap();
    assert_eq!(time["enough"], false);
    assert!(time["replications_needed"].as_u64().unwrap() > 3);

    // With the model's own 30 replications, nothing to warn about.
    let out = run(&[
        "run",
        example().to_str().unwrap(),
        "--output-dir",
        dir.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(!stdout.contains("aren't enough"), "{stdout}");
}

#[test]
fn run_writes_results_and_event_log() {
    let dir = temp_dir("files");
    let out = run(&[
        "run",
        example().to_str().unwrap(),
        "--replications",
        "4",
        "--output-dir",
        dir.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("4 replications"), "{stdout}");
    assert!(stdout.contains("time in system"), "{stdout}");

    let summary: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["format"], "simulancer-summary/0.1");
    assert_eq!(summary["replications"], 4);
    let busy = summary["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["object"] == "lathe" && m["measure"] == "busy")
        .unwrap();
    assert_eq!(busy["unit"], "fraction");
    assert_eq!(busy["n"], 4);
    assert!(busy["half_width"].as_f64().unwrap() > 0.0);

    let csv = fs::read_to_string(dir.join("replications.csv")).unwrap();
    assert!(csv.starts_with("replication,object_type,object,measure,unit,value\n"));
    assert!(csv.contains("\n4,server,lathe,busy,fraction,"));

    // The report embeds the results as JSON and needs nothing from the network.
    let report = fs::read_to_string(dir.join("report.html")).unwrap();
    let start = report
        .find(r#"<script type="application/json" id="data">"#)
        .unwrap();
    let json_start = report[start..].find('>').unwrap() + start + 1;
    let json_end = report[json_start..].find("</script>").unwrap() + json_start;
    let embedded: serde_json::Value = serde_json::from_str(&report[json_start..json_end]).unwrap();
    assert_eq!(embedded["model"], "conveyor-line");
    assert_eq!(
        embedded["measures"][0]["values"].as_array().unwrap().len(),
        4
    );
    assert!(!report.contains("__SIMULANCER_DATA__"));
    assert!(
        !report.replace(SVG_NS, "").contains("http"),
        "report must not load anything"
    );

    let events = fs::read_to_string(dir.join("events.jsonl")).unwrap();
    let mut lines = events.lines();
    let header: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(header["format"], "simulancer-events/0.1");
    assert_eq!(header["replication"], 1);
    assert_eq!(header["conveyors"][0]["capacity"], 24);
    let first: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(first["type"], "created");
    // Every line is valid JSON and time never goes backwards.
    let mut last = 0.0;
    for line in lines {
        let event: serde_json::Value = serde_json::from_str(line).unwrap();
        let t = event["t"].as_f64().unwrap();
        assert!(t >= last, "time went backwards at {line}");
        last = t;
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn results_dont_depend_on_thread_count() {
    let summary = |threads: &str| {
        let dir = temp_dir(&format!("threads-{threads}"));
        run(&[
            "run",
            example().to_str().unwrap(),
            "--replications",
            "6",
            "--threads",
            threads,
            "--output-dir",
            dir.to_str().unwrap(),
        ]);
        let files = [
            "summary.json",
            "replications.csv",
            "events.jsonl",
            "report.html",
        ]
        .map(|f| fs::read_to_string(dir.join(f)).unwrap());
        fs::remove_dir_all(dir).unwrap();
        files
    };
    assert_eq!(summary("1"), summary("4"));
}

#[test]
fn invalid_model_fails_with_located_error() {
    let dir = temp_dir("invalid");
    let model = dir.join("broken.yaml");
    let source = fs::read_to_string(example())
        .unwrap()
        .replace("to: mill", "to: mil");
    fs::write(&model, source).unwrap();
    let output = Command::new(BIN)
        .args(["run", model.to_str().unwrap(), "--output-dir"])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("goes to `mil`, which doesn't exist"),
        "{stderr}"
    );
    assert!(stderr.contains("broken.yaml:"), "{stderr}");
    assert!(!dir.join("summary.json").exists());
    fs::remove_dir_all(dir).unwrap();
}

/// The sink's throughput in summary.json: (value, unit).
fn shipping_throughput(dir: &Path) -> (f64, String) {
    let summary: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("summary.json")).unwrap()).unwrap();
    let m = summary["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["object"] == "shipping" && m["measure"] == "throughput")
        .unwrap()
        .clone();
    (
        m["mean"].as_f64().unwrap(),
        m["unit"].as_str().unwrap().to_string(),
    )
}

#[test]
fn throughput_per_hour_day_week_or_working_hour() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/assembly-line.yaml"),
    )
    .unwrap()
    .replace("event_log: jsonl", "event_log: none")
    .replace("throughput_per: two_shifts", "throughput_per: PER");
    let mut values = Vec::new();
    for per in ["hour", "day", "week", "two_shifts"] {
        let dir = temp_dir(&format!("per-{per}"));
        let model = dir.join("model.yaml");
        fs::write(&model, source.replace("PER", per)).unwrap();
        let out = dir.join("out");
        run(&[
            "run",
            model.to_str().unwrap(),
            "-r",
            "2",
            "-o",
            out.to_str().unwrap(),
        ]);
        values.push(shipping_throughput(&out));
        fs::remove_dir_all(&dir).unwrap();
    }
    let hour = values[0].0;
    let units: Vec<&str> = values.iter().map(|(_, u)| u.as_str()).collect();
    assert_eq!(
        units,
        ["parts/h", "parts/day", "parts/week", "parts/working h"]
    );
    // Same runs, other units. The measured period is four whole weeks of
    // which the two shifts are 80 of every 168 hours.
    for ((value, unit), factor) in values.iter().zip([1.0, 24.0, 168.0, 168.0 / 80.0]) {
        assert!(
            (value - hour * factor).abs() < 1e-9 * value.abs().max(1.0),
            "{unit}: {value}"
        );
    }
}

/// `simulancer test` passes when every expected result is met and fails
/// (exit 1) when one isn't; `validate` rejects expectations it can't read.
#[test]
fn test_checks_expected_results() {
    let dir = temp_dir("expect");
    let base = fs::read_to_string(example()).unwrap();
    let ok = run(&[
        "test",
        example().to_str().unwrap(),
        "--replications",
        "5",
        "--output-dir",
        dir.join("ok").to_str().unwrap(),
    ]);
    let stdout = String::from_utf8(ok.stdout).unwrap();
    assert!(stdout.contains("expected results: 3 of 3 met"), "{stdout}");
    let summary: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("ok/summary.json")).unwrap()).unwrap();
    assert_eq!(summary["expectations"].as_array().unwrap().len(), 3);
    assert_eq!(summary["expectations"][2]["met"], true);

    // A model whose line ships fewer than 60 an hour fails the test.
    let failing = dir.join("failing.yaml");
    fs::write(&failing, base.replace("at_least: 54 }", "at_least: 60 }")).unwrap();
    let out = Command::new(BIN)
        .args(["test", failing.to_str().unwrap(), "-r", "5", "-o"])
        .arg(dir.join("failing"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("expected results: 2 of 3 met"), "{stdout}");
    assert!(
        stdout.contains("expected the mean at least 60.0 /h · NOT MET"),
        "{stdout}"
    );

    // Limits in the wrong kind of unit, and unknown measures, are errors.
    for (from, to, message) in [
        (
            "at_least: 70%",
            "at_least: 0.7",
            "is a percentage, like `85%`, got `0.7`",
        ),
        (
            "at_most: 40 min",
            "at_most: 40 m",
            "is a time, like `30 min`",
        ),
        (
            "measure: busy",
            "measure: bussy",
            "`lathe` has no measure `bussy` (it has: idle, busy",
        ),
        ("check: interval", "check: everything", "`check` is `mean`"),
    ] {
        let bad = dir.join("bad.yaml");
        fs::write(&bad, base.replace(from, to)).unwrap();
        let out = Command::new(BIN)
            .args(["validate", bad.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(!out.status.success(), "{to}");
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(stderr.contains(message), "{to}: {stderr}");
    }
}
