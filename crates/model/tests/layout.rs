use outliers_sim_model::{LOOKS, Model, load_layout, load_str};

const MODEL: &str = include_str!("../../../examples/conveyor-line.yaml");
const LAYOUT: &str = include_str!("../../../examples/conveyor-line.layout.yaml");

fn model() -> Model {
    load_str(MODEL).unwrap()
}

fn errors(layout: &str) -> Vec<String> {
    match load_layout(layout, &model()) {
        Ok(_) => panic!("expected the layout to be rejected"),
        Err(err) => err
            .diagnostics()
            .iter()
            .map(|d| d.message.clone())
            .collect(),
    }
}

fn replace(source: &str, from: &str, to: &str) -> String {
    assert!(source.contains(from), "test bug: `{from}` not in layout");
    source.replacen(from, to, 1)
}

#[test]
fn example_layout_matches_its_model() {
    let (layout, warnings) = load_layout(LAYOUT, &model()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let names: Vec<&str> = layout.objects.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["arrivals", "lathe", "mill", "shipping"]);
    assert_eq!(layout.objects[1].model.as_deref(), Some("lathe"));
    let c2 = &layout.conveyors[1];
    assert_eq!(c2.path.len(), 3);
    assert!((c2.path_length() - 8.0).abs() < 1e-9);
}

#[test]
fn units_scale_positions() {
    let layout = replace(LAYOUT, "units: m", "units: cm");
    let (layout, warnings) = load_layout(&layout, &model()).unwrap();
    assert_eq!(layout.objects[1].position, [0.135, 0.0, 0.0]);
    // Every conveyor is now 100× shorter than in the model.
    assert_eq!(warnings.len(), 3);
    assert!(
        warnings[0]
            .message
            .contains("drawn 0.12 m long but is 12.00 m")
    );
}

#[test]
fn missing_and_unknown_names() {
    let errors = errors(&replace(LAYOUT, "  mill: ", "  mil: "));
    assert!(
        errors
            .iter()
            .any(|e| e.contains("`mil` isn't a source, server or sink")),
        "{errors:?}"
    );
    assert!(
        errors.iter().any(|e| e.contains("no position for `mill`")),
        "{errors:?}"
    );
}

#[test]
fn conveyor_listed_as_object() {
    let layout = replace(
        LAYOUT,
        "objects:",
        "objects:\n  c1: { position: [0, 0, 0] }",
    );
    let errors = errors(&layout);
    assert!(
        errors.iter().any(|e| e.contains("it's a conveyor")),
        "{errors:?}"
    );
}

#[test]
fn bad_paths_and_points() {
    let short = replace(
        LAYOUT,
        "c3: { path: [[21, 0, 5], [21, 0, 11]] }",
        "c3: { path: [[21, 0, 5]] }",
    );
    assert!(errors(&short)[0].contains("at least two points"));

    let flat = replace(LAYOUT, "position: [0, 0, 0]", "position: [0, 0]");
    assert!(errors(&flat)[0].contains("three numbers"));

    let repeated = replace(
        LAYOUT,
        "[[21, 0, 5], [21, 0, 11]]",
        "[[21, 0, 5], [21, 0, 5], [21, 0, 11]]",
    );
    assert!(errors(&repeated)[0].contains("two identical points"));
}

#[test]
fn unknown_keys_are_rejected() {
    let layout = replace(LAYOUT, "units: m", "unit: m");
    assert!(errors(&layout)[0].contains("unknown field `unit`"));
}

#[test]
fn looks_that_dont_fit_are_warnings() {
    // Not a look at all, and a look sources can't have: both drawn as the default.
    let layout = replace(LAYOUT, "model: lathe", "model: robot");
    let layout = replace(
        &layout,
        "arrivals: { position: [0, 0, 0], model: dock }",
        "arrivals: { position: [0, 0, 0], model: bin }",
    );
    let (placed, warnings) = load_layout(&layout, &model()).unwrap();
    let messages: Vec<&str> = warnings.iter().map(|w| w.message.as_str()).collect();
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(messages.iter().any(|m| m.starts_with("`lathe` can't be drawn as `robot`; it will look like a machine. A server can be drawn as: machine, lathe")), "{messages:?}");
    assert!(messages.iter().any(|m| m.starts_with("`arrivals` can't be drawn as `bin`; it will look like a dock. A source can be drawn as: dock, door, generic")), "{messages:?}");
    assert!(
        warnings.iter().all(|w| w.position.is_some()),
        "warnings point at the value"
    );
    // The value is kept as written.
    assert_eq!(placed.objects[1].model.as_deref(), Some("robot"));
}

#[test]
fn older_look_names_are_read() {
    let layout = replace(LAYOUT, "model: lathe", "model: machine_small");
    let (_, warnings) = load_layout(&layout, &model()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn looks_match_what_the_pages_draw() {
    // Every look the layout accepts is drawn by the web pages, for the same kinds of object.
    let art = include_str!("../../cli/src/web/art.js");
    let drawn: Vec<(String, String)> = art
        .lines()
        .zip(art.lines().skip(1))
        .filter_map(|(line, next)| {
            let name = line.strip_prefix("    ")?.strip_suffix(": {")?;
            let both = format!("{line} {next}");
            let sections = both.split("sections: [").nth(1)?.split(']').next()?;
            Some((name.to_string(), sections.replace(['"', ' '], "")))
        })
        .collect();
    let looks: Vec<(String, String)> = LOOKS
        .iter()
        .map(|(name, sections)| (name.to_string(), sections.join(",")))
        .collect();
    assert_eq!(drawn, looks);
}

#[test]
fn fleets_say_what_their_vehicles_look_like() {
    let model = load_str(include_str!("../../../examples/warehouse.yaml")).unwrap();
    let layout = include_str!("../../../examples/warehouse.layout.yaml");
    let (placed, warnings) = load_layout(layout, &model).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(placed.vehicles.len(), 1);
    assert_eq!(
        (
            placed.vehicles[0].name.as_str(),
            placed.vehicles[0].model.as_str()
        ),
        ("forklifts", "forklift")
    );

    // A look vehicles can't have, and a fleet the model doesn't have: warnings.
    let layout = replace(
        layout,
        "forklifts: { model: forklift }",
        "forklifts: { model: lathe }\n  tuggers: { model: agv }",
    );
    let (placed, warnings) = load_layout(&layout, &model).unwrap();
    let messages: Vec<&str> = warnings.iter().map(|w| w.message.as_str()).collect();
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(messages.iter().any(|m| m.starts_with("`forklifts` can't be drawn as `lathe`; its vehicles will look like a forklift. A fleet of vehicles can be drawn as: forklift, agv")), "{messages:?}");
    assert!(
        messages
            .iter()
            .any(|m| m.starts_with("`tuggers` isn't a fleet of vehicles in the model")),
        "{messages:?}"
    );
    assert_eq!(placed.vehicles.len(), 1, "the unknown fleet is left out");
}
