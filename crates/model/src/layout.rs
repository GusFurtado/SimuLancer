//! Layout files (`*.layout.yaml`): where objects and conveyors are drawn.
//! Queues aren't in the file: they're drawn straight between their objects.
//!
//! Only the diagram and the animation use layouts; the engine never reads them.
//! They're checked against the model so the pages can trust every name and path.

use serde::Deserialize;
use serde_saphyr::Spanned;

use crate::diagnostic::{Diagnostic, LoadError, Position};
use crate::raw::OrderedMap;
use crate::units::{DefaultUnits, Dimension, parse_quantity};
use crate::{Model, parse_yaml};
use outliers_sim_engine::spec::Node;

/// What a source, server, sink or fleet of vehicles can be drawn as (`model:`
/// in a layout), and which kinds of object can use it. The web pages draw them
/// (`crates/cli/src/web/art.js`); a test keeps the two lists in step.
pub const LOOKS: &[(&str, &[&str])] = &[
    ("machine", &["servers"]),
    ("lathe", &["servers"]),
    ("mill", &["servers"]),
    ("oven", &["servers"]),
    ("workstation", &["servers"]),
    ("counter", &["servers"]),
    ("kiosk", &["servers"]),
    ("dock", &["sources", "sinks"]),
    ("door", &["sources", "sinks"]),
    ("bin", &["sinks"]),
    ("generic", &["sources", "sinks"]),
    ("forklift", &["vehicles"]),
    ("agv", &["vehicles"]),
];

/// Older names for looks, still read.
const LOOK_ALIASES: &[(&str, &str)] = &[("machine_small", "machine")];

/// Half an object's size plus a gap: where connections start and end.
const OBJECT_EDGE: f64 = 0.7;

/// A queue's path: straight from one object's edge to the other's, moved
/// 0.5 m to the right of the way parts go when a queue runs the other way.
pub fn queue_path(a: [f64; 3], b: [f64; 3], reverse: bool) -> Vec<[f64; 3]> {
    let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
    let len = dx.hypot(dz);
    if len <= 2.0 * OBJECT_EDGE {
        // Objects (nearly) on top of each other: a short stub.
        return vec![a, [a[0] + 0.01, a[1], a[2]]];
    }
    let (ux, uz) = (dx / len, dz / len);
    let (ox, oz) = if reverse {
        (-uz * 0.5, ux * 0.5)
    } else {
        (0.0, 0.0)
    };
    vec![
        [
            a[0] + ux * OBJECT_EDGE + ox,
            a[1],
            a[2] + uz * OBJECT_EDGE + oz,
        ],
        [
            b[0] - ux * OBJECT_EDGE + ox,
            b[1],
            b[2] - uz * OBJECT_EDGE + oz,
        ],
    ]
}

/// A validated layout, in meters. Y is up; the floor is the X-Z plane.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Sources, servers and sinks, in model order.
    pub objects: Vec<PlacedObject>,
    /// In model order, queues included (with their computed paths).
    pub conveyors: Vec<PlacedConveyor>,
    /// Fleets the layout says how to draw, in model order.
    pub vehicles: Vec<DrawnFleet>,
}

/// How a fleet's vehicles are drawn.
#[derive(Debug, Clone)]
pub struct DrawnFleet {
    pub name: String,
    /// One of [`LOOKS`] for vehicles (e.g. `forklift`), as the layout gives it.
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct PlacedObject {
    pub name: String,
    pub position: [f64; 3],
    /// What the object is drawn as (one of [`LOOKS`], e.g. `lathe`), as the
    /// layout gives it. None: the pages draw the default for its kind.
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PlacedConveyor {
    pub name: String,
    pub path: Vec<[f64; 3]>,
}

impl PlacedConveyor {
    pub fn path_length(&self) -> f64 {
        self.path
            .windows(2)
            .map(|w| {
                let d: f64 = (0..3).map(|i| (w[1][i] - w[0][i]).powi(2)).sum();
                d.sqrt()
            })
            .sum()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLayout {
    /// The model file this layout belongs to; informational.
    #[allow(dead_code)]
    model: Option<String>,
    units: Option<Spanned<String>>,
    #[serde(default)]
    objects: OrderedMap<RawObject>,
    #[serde(default)]
    conveyors: OrderedMap<RawConveyorPath>,
    #[serde(default)]
    vehicles: OrderedMap<RawFleet>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFleet {
    model: Spanned<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawObject {
    position: Spanned<Vec<f64>>,
    model: Option<Spanned<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConveyorPath {
    path: Spanned<Vec<Vec<f64>>>,
}

/// Parses a layout and checks it against `model`. Returns the layout and
/// warnings (problems that don't stop drawing a run, like a drawn conveyor
/// whose length differs from the model's).
pub fn load_layout(source: &str, model: &Model) -> Result<(Layout, Vec<Diagnostic>), LoadError> {
    let raw: RawLayout = parse_yaml(source)?;
    let spec = &model.spec;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let error = |errors: &mut Vec<Diagnostic>, at: Option<Position>, message: String| {
        errors.push(Diagnostic {
            position: at,
            message,
        })
    };
    let pos = |s: &serde_saphyr::Location| Position::from_location(s);

    let scale = match &raw.units {
        None => 1.0,
        Some(unit) => match parse_quantity(
            &format!("1 {}", unit.value),
            Dimension::Length,
            &DefaultUnits::default(),
        ) {
            Ok(meters) => meters,
            Err(message) => {
                error(&mut errors, pos(&unit.referenced), message);
                1.0
            }
        },
    };

    let point = |errors: &mut Vec<Diagnostic>, values: &[f64], at: Option<Position>, what: &str| {
        if values.len() != 3 || values.iter().any(|v| !v.is_finite()) {
            errors.push(Diagnostic {
                position: at,
                message: format!("{what} must be three numbers [x, y, z]"),
            });
            None
        } else {
            Some([values[0] * scale, values[1] * scale, values[2] * scale])
        }
    };

    // Objects: every source, server and sink, and nothing else.
    let object_names: Vec<&str> = spec
        .sources
        .iter()
        .map(|s| s.name.as_str())
        .chain(spec.servers.iter().map(|s| s.name.as_str()))
        .chain(spec.sinks.iter().map(|s| s.name.as_str()))
        .collect();
    let mut objects = Vec::new();
    for name in &object_names {
        match raw.objects.0.iter().find(|(n, _)| n.value == *name) {
            Some((_, o)) => {
                if let Some(position) = point(
                    &mut errors,
                    &o.position.value,
                    pos(&o.position.referenced),
                    "`position`",
                ) {
                    if let Some(look) = &o.model {
                        let section = if spec.sources.iter().any(|s| s.name == *name) {
                            "sources"
                        } else if spec.servers.iter().any(|s| s.name == *name) {
                            "servers"
                        } else {
                            "sinks"
                        };
                        check_look(&mut warnings, name, section, look);
                    }
                    objects.push(PlacedObject {
                        name: name.to_string(),
                        position,
                        model: o.model.as_ref().map(|m| m.value.clone()),
                    });
                }
            }
            None => error(
                &mut errors,
                None,
                format!("`objects` has no position for `{name}`"),
            ),
        }
    }
    for (name, _) in &raw.objects.0 {
        if !object_names.contains(&name.value.as_str()) {
            let hint = if spec.conveyors.iter().any(|c| c.name == name.value) {
                " (it's a conveyor: put it under `conveyors` with a `path`)"
            } else {
                ""
            };
            error(
                &mut errors,
                pos(&name.referenced),
                format!(
                    "`{}` isn't a source, server or sink in the model{hint}",
                    name.value
                ),
            );
        }
    }

    // Conveyors: a path for each, which should be about as long as the model says.
    // Queues: a straight line between their objects.
    let mut conveyors = Vec::new();
    for c in &spec.conveyors {
        if !c.is_belt() {
            if let Some((name, _)) = raw.conveyors.0.iter().find(|(n, _)| n.value == c.name) {
                error(
                    &mut errors,
                    pos(&name.referenced),
                    format!(
                        "`{}` is a queue or transport, drawn straight between its objects, so remove it from `conveyors`",
                        c.name
                    ),
                );
            }
            let at = |node: Node| {
                let name = match node {
                    Node::Source(i) => &spec.sources[i].name,
                    Node::Server(i) => &spec.servers[i].name,
                    Node::Sink(i) => &spec.sinks[i].name,
                };
                objects.iter().find(|o| &o.name == name).map(|o| o.position)
            };
            if let (Some(a), Some(b)) = (at(c.from), at(c.to)) {
                // Two queues between the same objects in opposite directions
                // (a loop) are drawn side by side.
                let reverse = spec
                    .conveyors
                    .iter()
                    .any(|o| !o.is_belt() && o.from == c.to && o.to == c.from);
                conveyors.push(PlacedConveyor {
                    name: c.name.clone(),
                    path: queue_path(a, b, reverse),
                });
            }
            continue;
        }
        let Some((name, raw_path)) = raw.conveyors.0.iter().find(|(n, _)| n.value == c.name) else {
            error(
                &mut errors,
                None,
                format!("`conveyors` has no path for `{}`", c.name),
            );
            continue;
        };
        let at = pos(&raw_path.path.referenced);
        let points: Option<Vec<[f64; 3]>> = raw_path
            .path
            .value
            .iter()
            .map(|p| point(&mut errors, p, at, "each point of `path`"))
            .collect();
        let Some(points) = points else { continue };
        if points.len() < 2 {
            error(
                &mut errors,
                at,
                format!("conveyor `{}` needs a path of at least two points", c.name),
            );
            continue;
        }
        if points.windows(2).any(|w| w[0] == w[1]) {
            error(
                &mut errors,
                at,
                format!("conveyor `{}` has two identical points in a row", c.name),
            );
            continue;
        }
        let placed = PlacedConveyor {
            name: c.name.clone(),
            path: points,
        };
        let drawn = placed.path_length();
        if (drawn - c.length).abs() > 0.01 * c.length {
            warnings.push(Diagnostic {
                position: pos(&name.referenced),
                message: format!(
                    "conveyor `{}` is drawn {:.2} m long but is {:.2} m in the model; the animation will stretch to fit",
                    c.name, drawn, c.length
                ),
            });
        }
        conveyors.push(placed);
    }
    for (name, _) in &raw.conveyors.0 {
        if !spec.conveyors.iter().any(|c| c.name == name.value) {
            error(
                &mut errors,
                pos(&name.referenced),
                format!("`{}` isn't a conveyor in the model", name.value),
            );
        }
    }

    // Vehicles: how each fleet is drawn. A fleet the model doesn't have is
    // left out with a warning, so renaming one never stops a project opening.
    let mut vehicles = Vec::new();
    for fleet in &spec.vehicles {
        if let Some((_, f)) = raw.vehicles.0.iter().find(|(n, _)| n.value == fleet.name) {
            check_look(&mut warnings, &fleet.name, "vehicles", &f.model);
            vehicles.push(DrawnFleet {
                name: fleet.name.clone(),
                model: f.model.value.clone(),
            });
        }
    }
    for (name, _) in &raw.vehicles.0 {
        if !spec.vehicles.iter().any(|f| f.name == name.value) {
            warnings.push(Diagnostic {
                position: pos(&name.referenced),
                message: format!(
                    "`{}` isn't a fleet of vehicles in the model; it's ignored",
                    name.value
                ),
            });
        }
    }

    if errors.is_empty() {
        Ok((
            Layout {
                objects,
                conveyors,
                vehicles,
            },
            warnings,
        ))
    } else {
        errors.sort_by_key(|d| {
            d.position
                .map_or((true, 0, 0), |p| (false, p.line, p.column))
        });
        Err(LoadError::Invalid(errors))
    }
}

/// Warns when `look` isn't one an object in `section` can be drawn as; the
/// pages then draw the default, so it never stops a project from opening.
fn check_look(warnings: &mut Vec<Diagnostic>, name: &str, section: &str, look: &Spanned<String>) {
    let value = LOOK_ALIASES
        .iter()
        .find(|(old, _)| *old == look.value)
        .map_or(look.value.as_str(), |(_, new)| new);
    if LOOKS
        .iter()
        .any(|(l, s)| *l == value && s.contains(&section))
    {
        return;
    }
    let known: Vec<&str> = LOOKS
        .iter()
        .filter(|(_, s)| s.contains(&section))
        .map(|(l, _)| *l)
        .collect();
    let kind = match section {
        "vehicles" => "fleet of vehicles",
        _ => &section[..section.len() - 1],
    };
    let each = if section == "vehicles" {
        "its vehicles"
    } else {
        "it"
    };
    warnings.push(Diagnostic {
        position: Position::from_location(&look.referenced),
        message: format!(
            "`{}` can't be drawn as `{}`; {} will look like a {}. A {} can be drawn as: {}",
            name,
            look.value,
            each,
            known[0],
            kind,
            known.join(", ")
        ),
    });
}
