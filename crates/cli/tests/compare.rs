//! End-to-end tests of `simulancer compare`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_simulancer");

fn example(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
        .to_str()
        .unwrap()
        .to_string()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("simulancer-compare-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `compare` and returns comparison.json.
fn compare(base: &str, scenario: &str, replications: &str, name: &str) -> serde_json::Value {
    let dir = temp_dir(name);
    let output = Command::new(BIN)
        .args(["compare", base, scenario, "-r", replications, "-o"])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = fs::read_to_string(dir.join("comparison.json")).unwrap();
    fs::remove_dir_all(dir).unwrap();
    serde_json::from_str(&json).unwrap()
}

fn find<'a>(c: &'a serde_json::Value, object: &str, measure: &str) -> &'a serde_json::Value {
    c["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["object"] == object && m["measure"] == measure)
        .unwrap_or_else(|| panic!("no {object} {measure}"))
}

#[test]
fn a_model_compared_with_itself_is_identical() {
    // Common random numbers: the same model gives the same numbers, so every
    // difference is exactly zero.
    let base = example("conveyor-line.yaml");
    let c = compare(&base, &base, "4", "self");
    for m in c["measures"].as_array().unwrap() {
        assert_eq!(m["verdict"], "identical", "{m}");
        assert_eq!(m["difference"]["mean"], 0.0, "{m}");
    }
}

#[test]
fn longer_buffer_reduces_blocking() {
    let c = compare(
        &example("conveyor-line.yaml"),
        &example("conveyor-line-longer-c2.yaml"),
        "10",
        "buffer",
    );
    assert_eq!(c["common_random_numbers"], true);
    // More room between the lathe and the mill: the lathe is blocked less,
    // the line gets more out, and parts wait longer in the bigger buffer.
    assert_eq!(find(&c, "lathe", "blocked")["verdict"], "lower");
    assert_eq!(find(&c, "shipping", "throughput")["verdict"], "higher");
    assert_eq!(find(&c, "shipping", "time in system")["verdict"], "higher");
    // Pairing makes the interval much narrower than independent runs would.
    let blocked = find(&c, "lathe", "blocked");
    let paired = blocked["difference"]["half_width"].as_f64().unwrap();
    let unpaired = blocked["unpaired_half_width"].as_f64().unwrap();
    assert!(
        unpaired > 2.0 * paired,
        "paired {paired}, unpaired {unpaired}"
    );
}

#[test]
fn different_experiments_are_refused() {
    let dir = temp_dir("mismatch");
    let scenario = dir.join("other-seed.yaml");
    let source = fs::read_to_string(example("conveyor-line.yaml"))
        .unwrap()
        .replace("seed: 20261009", "seed: 7");
    fs::write(&scenario, source).unwrap();
    let output = Command::new(BIN)
        .args(["compare", &example("conveyor-line.yaml")])
        .arg(&scenario)
        .args(["-o"])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("same experiment settings"), "{stderr}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn routing_rule_comparison() {
    // Only the source's routing rule differs. Filling cell A's conveyor
    // first (first_available) makes parts wait far longer than balancing
    // by queue length (shortest_queue), for about the same throughput.
    let c = compare(
        &example("two-cells.yaml"),
        &example("two-cells-first-available.yaml"),
        "10",
        "routing",
    );
    let time = find(&c, "shipping", "time in system");
    assert_eq!(time["verdict"], "higher");
    assert!(
        time["difference"]["mean"].as_f64().unwrap() > 600.0,
        "{time}"
    );
    assert_eq!(find(&c, "mill_b", "idle")["verdict"], "higher");
    let throughput = find(&c, "shipping", "throughput");
    assert!(
        throughput["difference"]["mean"].as_f64().unwrap().abs() < 1.0,
        "{throughput}"
    );
}
