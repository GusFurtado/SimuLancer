use outliers_sim_model::{
    LoadError, Model, ScenarioError, is_scenario, load_scenario, load_str, parse_scenario,
};

const TWO_CELLS: &str = include_str!("../../../examples/two-cells.yaml");
const CONVEYOR_LINE: &str = include_str!("../../../examples/conveyor-line.yaml");

fn scenario(name: &str, base: &str, changes: &str) -> String {
    format!("format: outliers-sim-scenario/0.1\nname: {name}\nbase: {base}\nchanges:\n{changes}")
}

fn load(scenario_source: &str, base_source: &str) -> Result<Model, ScenarioError> {
    let s = parse_scenario(scenario_source).map_err(ScenarioError::Scenario)?;
    load_scenario(&s, base_source)
}

/// (line, message) of each error the scenario file is blamed for.
fn scenario_errors(scenario_source: &str, base_source: &str) -> Vec<(Option<u64>, String)> {
    match load(scenario_source, base_source) {
        Err(ScenarioError::Scenario(err)) => err
            .diagnostics()
            .iter()
            .map(|d| (d.position.map(|p| p.line), d.message.clone()))
            .collect(),
        Err(ScenarioError::Base(err)) => panic!("blamed the base: {err:?}"),
        Ok(_) => panic!("expected errors"),
    }
}

#[test]
fn a_scenario_equals_the_full_copy_it_replaces() {
    // The same model written out in full: the old way to make a scenario.
    let copy = TWO_CELLS
        .replace("name: two-cells\n", "name: two-cells-first-available\n")
        .replace(
            "output: { rule: shortest_queue }",
            "output: { rule: first_available }",
        );
    let full = load_str(&copy).unwrap();
    let s = scenario(
        "two-cells-first-available",
        "two-cells.yaml",
        "  sources.arrivals.output: { rule: first_available }\n",
    );
    let from_changes = load(&s, TWO_CELLS).unwrap();
    assert_eq!(from_changes.name, "two-cells-first-available");
    assert_eq!(from_changes.spec, full.spec);
}

#[test]
fn several_changes_including_list_items_and_new_keys() {
    let s = scenario(
        "tweaked",
        "conveyor-line.yaml",
        r#"  conveyors.c2.length: "16 m"
  servers.mill.failures.0.downtime: "uniform(5 min, 15 min)"
  servers.lathe.capacity: 2
  experiment.replications: 5
"#,
    );
    let m = load(&s, CONVEYOR_LINE).unwrap().spec;
    assert_eq!(m.conveyors[1].length, 16.0);
    assert_eq!(m.conveyors[1].capacity, 32);
    assert_eq!(m.servers[0].capacity, 2); // a key the base didn't have
    assert_eq!(m.experiment.replications, 5);
    let base = load_str(CONVEYOR_LINE).unwrap().spec;
    assert_eq!(
        m.servers[1].failures[0].uptime,
        base.servers[1].failures[0].uptime
    );
    assert_ne!(
        m.servers[1].failures[0].downtime,
        base.servers[1].failures[0].downtime
    );
}

#[test]
fn errors_point_at_the_change_in_the_scenario_file() {
    // A bad value: reported at the change's line, with the model's message.
    let s = scenario(
        "typo",
        "two-cells.yaml",
        "  sources.arrivals.output: { rule: first_availble }\n",
    );
    let errors = scenario_errors(&s, TWO_CELLS);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].0, Some(5));
    assert!(
        errors[0].1.contains("unknown variant `first_availble`"),
        "{errors:?}"
    );

    // A unit error deep inside a changed value.
    let s = scenario(
        "units",
        "conveyor-line.yaml",
        "  conveyors.c2.speed: \"5 min\"\n",
    );
    let errors = scenario_errors(&s, CONVEYOR_LINE);
    assert_eq!(errors[0].0, Some(5));
    assert!(
        errors[0].1.contains("is a time, but a speed is expected"),
        "{errors:?}"
    );
}

#[test]
fn bad_paths_are_reported_with_what_exists() {
    let s = scenario(
        "paths",
        "two-cells.yaml",
        "  servers.mil_b.process_time: \"1 min\"\n  servers.mill_b.failures.3.uptime: \"1 h\"\n  name: renamed\n",
    );
    let errors = scenario_errors(&s, TWO_CELLS);
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert!(
        errors[0]
            .1
            .contains("`servers` has no `mil_b` (it has: mill_a, mill_b, inspect)")
    );
    assert!(
        errors[1]
            .1
            .contains("is a list of 1; use an index from 0 to 0")
    );
    assert!(
        errors[2]
            .1
            .contains("set it at the top of the scenario file")
    );
    assert_eq!(
        errors.iter().map(|e| e.0).collect::<Vec<_>>(),
        [Some(5), Some(6), Some(7)]
    );
}

#[test]
fn errors_in_the_base_blame_the_base() {
    let broken_base = TWO_CELLS.replace("to: mill_a,", "to: mil_a,");
    let s = scenario("x", "two-cells.yaml", "  experiment.replications: 2\n");
    assert!(matches!(
        load(&s, &broken_base),
        Err(ScenarioError::Base(LoadError::Invalid(_)))
    ));
}

#[test]
fn scenario_files_are_recognized() {
    let s = scenario("x", "two-cells.yaml", "  experiment.replications: 2\n");
    assert!(is_scenario(&s));
    assert!(!is_scenario(TWO_CELLS));
    let wrong = s.replace("outliers-sim-scenario/0.1", "outliers-sim-scenario/9");
    assert!(parse_scenario(&wrong).is_err());
}

#[test]
fn writing_a_model_back_as_yaml_keeps_it_identical() {
    // Every example: parse to a tree, write it out, load both, compare.
    for (name, source) in [("conveyor-line", CONVEYOR_LINE), ("two-cells", TWO_CELLS)] {
        let tree = outliers_sim_model::to_tree(source).unwrap();
        let written = outliers_sim_model::to_yaml(&tree);
        let original = load_str(source).unwrap();
        let reloaded =
            load_str(&written).unwrap_or_else(|e| panic!("{name}: {}", e.render(name, &written)));
        assert_eq!(reloaded.spec, original.spec, "{name}");
        assert_eq!(reloaded.name, original.name);
        assert_eq!(
            outliers_sim_model::to_tree(&written).unwrap(),
            tree,
            "{name}"
        );
    }
}
