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

/// A Parquet file's rows, each as a JSON object of its non-null columns,
/// and its `simulancer` metadata.
fn parquet_rows(path: &Path) -> (Vec<serde_json::Value>, Option<String>) {
    use parquet::file::reader::{FileReader, SerializedFileReader};
    use parquet::record::Field;
    let reader = SerializedFileReader::new(fs::File::open(path).unwrap()).unwrap();
    let metadata = reader
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .and_then(|kv| kv.iter().find(|k| k.key == "simulancer"))
        .and_then(|k| k.value.clone());
    let rows = reader
        .get_row_iter(None)
        .unwrap()
        .map(|row| {
            let mut object = serde_json::Map::new();
            for (name, field) in row.unwrap().get_column_iter() {
                let value = match field {
                    Field::Null => continue,
                    Field::Double(v) => serde_json::json!(v),
                    Field::Long(v) => serde_json::json!(v),
                    Field::Str(v) => serde_json::json!(v),
                    other => panic!("unexpected {other:?} in {name}"),
                };
                object.insert(name.clone(), value);
            }
            serde_json::Value::Object(object)
        })
        .collect();
    (rows, metadata)
}

/// events.parquet holds the same events as events.jsonl, row for row, and
/// replications.parquet the same values as replications.csv.
#[test]
fn parquet_files_match_the_text_ones() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut types = std::collections::BTreeSet::new();
    for name in [
        "assembly-line",
        "bank-branch",
        "bike-assembly",
        "conveyor-line",
        "two-cells",
        "warehouse",
    ] {
        // Three days are enough to see every kind of event, and keep the
        // comparison quick in a debug build.
        let source: String = fs::read_to_string(examples.join(format!("{name}.yaml")))
            .unwrap()
            .lines()
            .map(|line| match line.trim_start() {
                l if l.starts_with("run_length:") => "  run_length: \"3 d\"\n".to_string(),
                l if l.starts_with("warmup:") => "  warmup: \"1 h\"\n".to_string(),
                _ => format!("{line}\n"),
            })
            .collect();
        let both = "event_log: [jsonl, parquet]";
        let source = if source.contains("event_log: jsonl") {
            source.replace("event_log: jsonl", both)
        } else if source.contains("\noutputs:\n") {
            source.replace("\noutputs:\n", &format!("\noutputs:\n  {both}\n"))
        } else {
            format!("{source}\noutputs:\n  {both}\n")
        };
        let dir = temp_dir(&format!("parquet-{name}"));
        let model = dir.join("model.yaml");
        fs::write(&model, source).unwrap();
        let out = dir.join("out");
        run(&[
            "run",
            model.to_str().unwrap(),
            "--replications",
            "1",
            "--output-dir",
            out.to_str().unwrap(),
        ]);

        let jsonl = fs::read_to_string(out.join("events.jsonl")).unwrap();
        let mut lines = jsonl.lines();
        let header: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        let (rows, metadata) = parquet_rows(&out.join("events.parquet"));
        let metadata: serde_json::Value = serde_json::from_str(&metadata.unwrap()).unwrap();
        assert_eq!(metadata, header, "{name}: the header is in the metadata");
        let events: Vec<serde_json::Value> = lines
            .map(|line| {
                let mut event: serde_json::Value = serde_json::from_str(line).unwrap();
                // A null in events.jsonl is a null column in events.parquet.
                event.as_object_mut().unwrap().retain(|_, v| !v.is_null());
                event
            })
            .collect();
        assert_eq!(rows.len(), events.len(), "{name}: as many events");
        for (i, (row, event)) in rows.iter().zip(&events).enumerate() {
            // serde_json's default float parsing can be one ulp off, so
            // attribute values (any double) are compared within that.
            let (mut row, mut event) = (row.clone(), event.clone());
            if let (Some(a), Some(b)) = (
                row.as_object_mut().unwrap().remove("value"),
                event.as_object_mut().unwrap().remove("value"),
            ) {
                let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
                assert!(
                    (a - b).abs() <= 1e-15 * a.abs().max(1.0),
                    "{name}: event {}: {a} vs {b}",
                    i + 1
                );
            }
            assert_eq!(row, event, "{name}: event {}", i + 1);
            types.insert(row["type"].as_str().unwrap().to_string());
        }

        let csv = fs::read_to_string(out.join("replications.csv")).unwrap();
        let (rows, metadata) = parquet_rows(&out.join("replications.parquet"));
        assert!(metadata.unwrap().contains("simulancer-replications/0.1"));
        let csv_rows: Vec<&str> = csv.lines().skip(1).collect();
        assert_eq!(rows.len(), csv_rows.len(), "{name}: as many values");
        for (row, line) in rows.iter().zip(csv_rows) {
            let cells: Vec<&str> = line.split(',').collect();
            assert_eq!(row["replication"], cells[0].parse::<i64>().unwrap());
            assert_eq!(row["object_type"], cells[1]);
            assert_eq!(row["object"], cells[2]);
            assert_eq!(row["measure"], cells[3]);
            assert_eq!(row["unit"], cells[4]);
            match cells[5] {
                "" => assert!(row.get("value").is_none(), "{line}"),
                v => assert_eq!(row["value"].as_f64().unwrap(), v.parse::<f64>().unwrap()),
            }
        }
        fs::remove_dir_all(dir).unwrap();
    }
    // The examples between them cover most kinds of event.
    for kind in [
        "created",
        "entered_conveyor",
        "reached_end",
        "delivered",
        "started_process",
        "finished_process",
        "exited",
        "failed",
        "repaired",
        "resource_taken",
        "resource_returned",
        "vehicle_trip",
        "assembled",
        "set",
        "gave_up",
        "shift_ended",
        "shift_started",
    ] {
        assert!(types.contains(kind), "no example has `{kind}`: {types:?}");
    }
}

/// `event_log: parquet` alone writes no events.jsonl, and a format that
/// isn't one is an error that says which there are.
#[test]
fn event_log_in_parquet_only() {
    let source = fs::read_to_string(example())
        .unwrap()
        .replace("event_log: jsonl", "event_log: parquet");
    assert!(source.contains("event_log: parquet"));
    let dir = temp_dir("parquet-only");
    let model = dir.join("model.yaml");
    fs::write(&model, &source).unwrap();
    let out = dir.join("out");
    run(&[
        "run",
        model.to_str().unwrap(),
        "--replications",
        "1",
        "--output-dir",
        out.to_str().unwrap(),
    ]);
    assert!(out.join("events.parquet").exists());
    assert!(!out.join("events.jsonl").exists());

    fs::write(
        &model,
        source.replace("event_log: parquet", "event_log: csv"),
    )
    .unwrap();
    let output = Command::new(BIN)
        .args(["validate", model.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown event log format `csv`: none, jsonl or parquet"),
        "{stderr}"
    );
    fs::remove_dir_all(dir).unwrap();
}
