//! Every example's results, pinned: running an example with its own seed has
//! to give exactly the `summary.json` kept in `tests/pinned/`. A change in the
//! engine, a distribution or the statistics that moves any number fails here.
//!
//! When a change is meant to move results (a bug fix, say), regenerate the
//! copies in the same pull request, so the diff shows what moved:
//!
//! ```sh
//! UPDATE_PINNED=1 cargo test -p simulancer --test pinned
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_simulancer");

/// The line naming the version that ran: it changes with every release
/// without moving any number, so it's left out of the comparison.
const VERSION_LINE: &str = "  \"simulancer_version\": ";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every model and scenario in `examples/`, sorted.
fn examples() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(repo().join("examples"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_str().unwrap();
            name.ends_with(".yaml") && !name.ends_with(".layout.yaml") && name != "templates.yaml"
        })
        .collect();
    files.sort();
    files
}

fn without_version(summary: &str) -> String {
    summary
        .lines()
        .filter(|l| !l.starts_with(VERSION_LINE))
        .map(|l| format!("{l}\n"))
        .collect()
}

/// The measures whose numbers differ, for a readable failure.
fn moved(pinned: &str, now: &str) -> Vec<String> {
    let parse = |text: &str| -> Vec<serde_json::Value> {
        serde_json::from_str::<serde_json::Value>(text).unwrap()["measures"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    let (pinned, now) = (parse(pinned), parse(now));
    let key = |m: &serde_json::Value| {
        let text = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
        format!("{} {}", text(&m["object"]), text(&m["measure"]))
    };
    let mut lines = Vec::new();
    for m in &now {
        match pinned.iter().find(|p| key(p) == key(m)) {
            Some(p) if p == m => {}
            Some(p) => lines.push(format!("{}: mean {} -> {}", key(m), p["mean"], m["mean"])),
            None => lines.push(format!("{}: new", key(m))),
        }
    }
    for p in &pinned {
        if !now.iter().any(|m| key(m) == key(p)) {
            lines.push(format!("{}: gone", key(p)));
        }
    }
    lines
}

#[test]
fn examples_give_their_pinned_results() {
    let update = std::env::var_os("UPDATE_PINNED").is_some();
    let pinned_dir = repo().join("tests/pinned");
    let out = std::env::temp_dir().join(format!("simulancer-pinned-{}", std::process::id()));
    let mut failures = Vec::new();
    let mut names = Vec::new();
    for model in examples() {
        let name = model.file_stem().unwrap().to_str().unwrap().to_string();
        let dir = out.join(&name);
        let output = Command::new(BIN)
            .args(["run", model.to_str().unwrap(), "--output-dir"])
            .arg(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "simulancer run {name} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let now = without_version(&fs::read_to_string(dir.join("summary.json")).unwrap());
        let file = pinned_dir.join(format!("{name}.summary.json"));
        if update {
            fs::create_dir_all(&pinned_dir).unwrap();
            fs::write(&file, &now).unwrap();
        } else {
            match fs::read_to_string(&file) {
                Ok(pinned) if pinned == now => {}
                Ok(pinned) => {
                    let mut why = moved(&pinned, &now);
                    if why.is_empty() {
                        why.push("the warm-up check or the experiment changed".into());
                    } else if why.len() > 8 {
                        let more = why.len() - 8;
                        why.truncate(8);
                        why.push(format!("and {more} more"));
                    }
                    failures.push(format!("{name}:\n    {}", why.join("\n    ")));
                }
                Err(_) => failures.push(format!("{name}: no pinned copy at {}", file.display())),
            }
        }
        names.push(format!("{name}.summary.json"));
    }
    let _ = fs::remove_dir_all(&out);
    // A pinned copy whose example is gone.
    for entry in fs::read_dir(&pinned_dir).unwrap().flatten() {
        let file = entry.file_name().into_string().unwrap();
        if !names.contains(&file) {
            if update {
                fs::remove_file(entry.path()).unwrap();
            } else {
                failures.push(format!("{file}: pinned, but there's no such example"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "results moved from the pinned copies:\n  {}\n\nIf the change is meant to move \
         them, run `UPDATE_PINNED=1 cargo test -p simulancer --test pinned` and say so in \
         the pull request.",
        failures.join("\n  ")
    );
}
