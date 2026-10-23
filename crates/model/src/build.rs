//! Validation: turns a [`RawModel`] into an engine [`Spec`], collecting every
//! problem instead of stopping at the first one.

use std::collections::HashMap;

use outliers_sim_engine::SimTime;
use outliers_sim_engine::spec::{
    Arrivals, Attribute, Changeover, ConnectionKind, Conveyor, Count, Distribution, Experiment,
    Expr, Failure, FailureClock, Fleet, InputRule, MAX_ATTRIBUTES, Node, OnFailure, OrderQuantity,
    PacedLine, PerType, QueueOrder, Resource, Restock, Routing, Schedule, Server, ServerKind, Sink,
    Source, Spec, WhenBlocked,
};
use serde_saphyr::Spanned;

use crate::ThroughputPer;
use crate::diagnostic::{Diagnostic, Position};
use crate::expr::{constant, parse_time_distribution};
use crate::formula::{self, Kind as FormulaKind, Names};
use crate::raw::{self, RawModel, Scalar};
use crate::units::{DefaultUnits, Dimension, convert_number, parse_quantity};
use crate::{FORMAT, Model, Outputs, Words};

pub(crate) fn build(raw: RawModel) -> Result<Model, Vec<Diagnostic>> {
    let mut b = Builder::default();

    if raw.format.value != FORMAT {
        b.error(
            &raw.format,
            format!(
                "unsupported format `{}` (this version reads `{FORMAT}`)",
                raw.format.value
            ),
        );
    }
    b.check_name(&raw.name);
    if let Some(units) = &raw.units {
        b.defaults.time = b.default_unit(&units.time, Dimension::Time);
        b.defaults.length = b.default_unit(&units.length, Dimension::Length);
    }

    // Names. Entity types have their own namespace; sources, servers,
    // conveyors, queues and sinks share one, since connections refer to them
    // by name.
    let mut entity_index = HashMap::new();
    for (i, (name, _)) in raw.entities.0.iter().enumerate() {
        b.check_name(name);
        entity_index.insert(name.value.as_str(), i);
    }
    let entity_names: Vec<&str> = raw
        .entities
        .0
        .iter()
        .map(|(n, _)| n.value.as_str())
        .collect();
    let types = Types {
        index: &entity_index,
        names: &entity_names,
    };

    // Attributes have their own namespace: only formulas and `sets` name them.
    let mut attributes = Vec::new();
    let mut attribute_kinds: Vec<(&str, FormulaKind)> = Vec::new();
    for (i, (name, a)) in raw.attributes.0.iter().enumerate() {
        b.check_name(name);
        if let Some(message) = formula::reserved(&name.value) {
            b.error(name, message);
        }
        if i == MAX_ATTRIBUTES {
            b.error(
                name,
                format!("a model can have at most {MAX_ATTRIBUTES} attributes"),
            );
        }
        let default = match &a.default.value {
            Scalar::Number(n) if n.is_finite() => Some((*n, FormulaKind::Number)),
            Scalar::Text(t) => parse_quantity(t, Dimension::Time, &DefaultUnits::default())
                .map(|v| (v, FormulaKind::Time))
                .ok()
                .or_else(|| {
                    t.trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|v| v.is_finite())
                        .map(|v| (v, FormulaKind::Number))
                }),
            Scalar::Number(_) => None,
        };
        match default {
            Some((value, kind)) => {
                attributes.push(Attribute {
                    name: name.value.clone(),
                    time: kind == FormulaKind::Time,
                    default: value,
                });
                attribute_kinds.push((&name.value, kind));
            }
            None => b.error(
                &a.default,
                format!(
                    "the default of `{}` is a plain number (`0`) or a time (`0 s`)",
                    name.value
                ),
            ),
        }
    }
    let names = Names {
        attributes: &attribute_kinds,
        entities: &entity_names,
    };

    // Schedules have their own namespace: only `schedule:` fields name them.
    let mut schedules = Vec::new();
    for (name, s) in &raw.schedules.0 {
        b.check_name(name);
        if let Some(periods) = b.schedule(name, s) {
            schedules.push(Schedule {
                name: name.value.clone(),
                periods,
                finish_work: s.finish_work,
            });
        }
    }
    let schedule_names: Vec<&str> = raw
        .schedules
        .0
        .iter()
        .map(|(n, _)| n.value.as_str())
        .collect();
    let mut objects: HashMap<&str, Object> = HashMap::new();
    let all_names = [
        (
            raw.sources.0.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            Kind::Source,
        ),
        (raw.servers.0.iter().map(|(n, _)| n).collect(), Kind::Server),
        (
            raw.conveyors.0.iter().map(|(n, _)| n).collect(),
            Kind::Conveyor,
        ),
        (raw.queues.0.iter().map(|(n, _)| n).collect(), Kind::Queue),
        (
            raw.transports.0.iter().map(|(n, _)| n).collect(),
            Kind::Transport,
        ),
        (raw.vehicles.0.iter().map(|(n, _)| n).collect(), Kind::Fleet),
        (raw.sinks.0.iter().map(|(n, _)| n).collect(), Kind::Sink),
        (
            raw.resources.0.iter().map(|(n, _)| n).collect(),
            Kind::Resource,
        ),
    ];
    for (names, kind) in &all_names {
        for (i, name) in names.iter().enumerate() {
            b.check_name(name);
            match objects.get(name.value.as_str()) {
                Some(existing) => b.error(
                    name,
                    format!(
                        "`{}` is already used by a {}; names must be unique across sources, servers, conveyors, queues, transports, vehicles, sinks and resources",
                        name.value,
                        existing.kind.describe()
                    ),
                ),
                None => {
                    objects.insert(&name.value, Object { kind: *kind, index: i });
                }
            }
        }
    }

    // Conveyors first, so sources and servers can find their connections.
    let mut source_outputs: Vec<Vec<usize>> = vec![Vec::new(); raw.sources.0.len()];
    let mut server_inputs: Vec<Vec<usize>> = vec![Vec::new(); raw.servers.0.len()];
    let mut server_outputs: Vec<Vec<usize>> = vec![Vec::new(); raw.servers.0.len()];
    let mut sink_inputs: Vec<Vec<usize>> = vec![Vec::new(); raw.sinks.0.len()];
    let mut conveyors = Vec::new();
    // A conveyor pointing at a misspelled name would also leave its intended
    // target "unconnected"; skip those follow-on errors and report only the cause.
    let mut endpoints_ok = true;
    for (i, (name, c)) in raw.conveyors.0.iter().enumerate() {
        // Accumulating is the only kind so far; a new kind makes this fail to compile.
        let raw::ConveyorKind::Accumulating = c.kind;
        let from = b.endpoint(&objects, "conveyor", name, &c.from, End::From);
        let to = b.endpoint(&objects, "conveyor", name, &c.to, End::To);
        endpoints_ok &= from.is_some() && to.is_some();
        match from {
            Some(Node::Source(s)) => source_outputs[s].push(i),
            Some(Node::Server(s)) => server_outputs[s].push(i),
            _ => {}
        }
        match to {
            Some(Node::Server(s)) => server_inputs[s].push(i),
            Some(Node::Sink(s)) => sink_inputs[s].push(i),
            _ => {}
        }
        let length = b.positive_quantity(&c.length, Dimension::Length, "length");
        let speed = b.positive_quantity(&c.speed, Dimension::Speed, "speed");
        let spacing = b.positive_quantity(&c.spacing, Dimension::Length, "spacing");
        let capacity = match (length, spacing) {
            (Some(length), Some(spacing)) => {
                // Tolerance so that e.g. 0.3 m / 0.1 m counts as 3, not 2.
                let fit = (length / spacing + 1e-9).floor();
                if fit < 1.0 {
                    b.error(
                        &c.spacing,
                        format!(
                            "conveyor `{}` is shorter than one spacing, so it can't hold any parts",
                            name.value
                        ),
                    );
                    None
                } else {
                    Some(fit.min(u32::MAX as f64) as u32)
                }
            }
            _ => None,
        };
        if let (Some(from), Some(to), Some(length), Some(speed), Some(spacing), Some(capacity)) =
            (from, to, length, speed, spacing, capacity)
        {
            conveyors.push(Conveyor {
                name: name.value.clone(),
                kind: ConnectionKind::Conveyor,
                from,
                to,
                length,
                speed,
                spacing,
                capacity,
                target_wait: None,
                starts_with: None,
                order: QueueOrder::FirstIn,
            });
        }
    }

    // Queues come after the conveyors in `Spec::conveyors`.
    for (j, (name, q)) in raw.queues.0.iter().enumerate() {
        let i = raw.conveyors.0.len() + j;
        let from = b.endpoint(&objects, "queue", name, &q.from, End::From);
        let to = b.endpoint(&objects, "queue", name, &q.to, End::To);
        endpoints_ok &= from.is_some() && to.is_some();
        match from {
            Some(Node::Source(s)) => source_outputs[s].push(i),
            Some(Node::Server(s)) => server_outputs[s].push(i),
            _ => {}
        }
        match to {
            Some(Node::Server(s)) => server_inputs[s].push(i),
            Some(Node::Sink(s)) => sink_inputs[s].push(i),
            _ => {}
        }
        let capacity = match &q.capacity {
            Some(c) if c.value == 0 => {
                b.error(
                    c,
                    format!(
                        "queue `{}` needs room for at least one part; leave `capacity` out for no limit",
                        name.value
                    ),
                );
                None
            }
            Some(c) => Some(Some(c.value)),
            None => Some(None),
        };
        let target_wait = match &q.target_wait {
            None => Some(None),
            Some(t) => b
                .positive_quantity(t, Dimension::Time, "target_wait")
                .map(Some),
        };
        let order = match &q.order {
            None => Some(QueueOrder::FirstIn),
            Some(order) => b.queue_order(order, &names),
        };
        if let (Some(from), Some(to), Some(capacity), Some(target_wait), Some(order)) =
            (from, to, capacity, target_wait, order)
        {
            conveyors.push(Conveyor {
                order,
                ..Conveyor::queue(name.value.clone(), from, to, capacity, target_wait)
            });
        }
    }

    // Vehicles: where they start, how fast, how long to load and unload.
    let mut vehicles = Vec::new();
    for (name, f) in &raw.vehicles.0 {
        if f.count.value == 0 {
            b.error(
                &f.count,
                format!("vehicles `{}` need a count of at least 1", name.value),
            );
        }
        let speed = b.positive_quantity(&f.speed, Dimension::Speed, "speed");
        let zero = Spanned {
            value: Scalar::Text("0 s".into()),
            referenced: name.referenced,
            defined: name.defined,
        };
        let load_time = b.distribution(f.load_time.as_ref().unwrap_or(&zero));
        let unload_time = b.distribution(f.unload_time.as_ref().unwrap_or(&zero));
        let home = match objects.get(f.home.value.as_str()) {
            Some(Object {
                kind: Kind::Source,
                index,
            }) => Some(Node::Source(*index)),
            Some(Object {
                kind: Kind::Server,
                index,
            }) => Some(Node::Server(*index)),
            Some(Object {
                kind: Kind::Sink,
                index,
            }) => Some(Node::Sink(*index)),
            _ => {
                b.error(
                    &f.home,
                    format!(
                        "`home` is a source, server or sink, and `{}` isn't one",
                        f.home.value
                    ),
                );
                None
            }
        };
        if let (Some(speed), Some(load_time), Some(unload_time), Some(home)) =
            (speed, load_time, unload_time, home)
        {
            vehicles.push(Fleet {
                name: name.value.clone(),
                count: f.count.value.max(1),
                speed,
                load_time,
                unload_time,
                home,
            });
        }
    }

    // Transports come after the queues in `Spec::conveyors`.
    for (k, (name, t)) in raw.transports.0.iter().enumerate() {
        let i = raw.conveyors.0.len() + raw.queues.0.len() + k;
        let from = b.endpoint(&objects, "transport", name, &t.from, End::From);
        let to = b.endpoint(&objects, "transport", name, &t.to, End::To);
        endpoints_ok &= from.is_some() && to.is_some();
        match from {
            Some(Node::Source(s)) => source_outputs[s].push(i),
            Some(Node::Server(s)) => server_outputs[s].push(i),
            _ => {}
        }
        match to {
            Some(Node::Server(s)) => server_inputs[s].push(i),
            Some(Node::Sink(s)) => sink_inputs[s].push(i),
            _ => {}
        }
        let fleet = raw
            .vehicles
            .0
            .iter()
            .position(|(n, _)| n.value == t.by.value);
        if fleet.is_none() {
            b.error(
                &t.by,
                format!(
                    "`by` names vehicles, and there are no vehicles `{}`",
                    t.by.value
                ),
            );
        }
        let distance = b.positive_quantity(&t.distance, Dimension::Length, "distance");
        let capacity = match &t.capacity {
            Some(c) if c.value == 0 => {
                b.error(c, format!("transport `{}` needs room for at least one part; leave `capacity` out for no limit", name.value));
                None
            }
            Some(c) => Some(Some(c.value)),
            None => Some(None),
        };
        if let (Some(from), Some(to), Some(fleet), Some(distance), Some(capacity)) =
            (from, to, fleet, distance, capacity)
        {
            conveyors.push(Conveyor::transport(
                name.value.clone(),
                from,
                to,
                capacity,
                fleet,
                distance,
            ));
        }
    }

    let conveyor_names: Vec<&str> = raw
        .conveyors
        .0
        .iter()
        .map(|(n, _)| n.value.as_str())
        .chain(raw.queues.0.iter().map(|(n, _)| n.value.as_str()))
        .chain(raw.transports.0.iter().map(|(n, _)| n.value.as_str()))
        .collect();

    let mut sources = Vec::new();
    for (i, (name, s)) in raw.sources.0.iter().enumerate() {
        let creates = b.creates(name, &s.creates, &types);
        let interarrival = match (&s.interarrival, &s.arrivals_per_hour) {
            _ if s.restocks.is_some() => b.restock(name, s, &raw).map(Arrivals::Restock),
            (Some(gap), None) => b.distribution(gap).map(Arrivals::Interarrival),
            (None, Some(profile)) => b.arrival_profile(name, profile).map(Arrivals::Profile),
            (Some(gap), Some(_)) => {
                b.error(
                    gap,
                    format!(
                        "source `{}` has both `interarrival` and `arrivals_per_hour`; keep one",
                        name.value
                    ),
                );
                None
            }
            (None, None) => {
                b.error(
                    name,
                    format!(
                        "source `{}` needs `interarrival` (time between arrivals), `arrivals_per_hour` (a rate by time of day) or `restocks` (a queue it keeps stocked)",
                        name.value
                    ),
                );
                None
            }
        };
        let group = match &s.group {
            None => Some(None),
            Some(g) => b.count(g, "`group`").map(Some),
        };
        let sets = b.sets(&s.sets, &names);
        let schedule = b.schedule_ref(name, &s.schedule, &schedule_names);
        let output = endpoints_ok
            .then(|| {
                b.output_rule(
                    name,
                    "source",
                    &s.output,
                    &source_outputs[i],
                    &conveyor_names,
                    &types,
                    &names,
                )
            })
            .flatten();
        if let (
            Some(creates),
            Some(interarrival),
            Some(group),
            Some((outputs, routing)),
            Some(schedule),
            Some(sets),
        ) = (creates, interarrival, group, output, schedule, sets)
        {
            sources.push(Source {
                name: name.value.clone(),
                creates,
                arrivals: interarrival,
                group,
                when_blocked: match s.when_blocked {
                    raw::WhenBlocked::Wait => WhenBlocked::Wait,
                    raw::WhenBlocked::Drop => WhenBlocked::Drop,
                },
                outputs,
                routing,
                schedule,
                sets,
            });
        }
    }

    let mut resources = Vec::new();
    for (name, r) in &raw.resources.0 {
        let schedule = b.schedule_ref(name, &r.schedule, &schedule_names).flatten();
        if r.capacity.value == 0 {
            b.error(
                &r.capacity,
                format!("resource `{}` needs a capacity of at least 1", name.value),
            );
        }
        let mut order = Vec::new();
        for server in r.order.iter().flat_map(|o| &o.value) {
            let needs_it =
                |s: &raw::RawServer| s.needs.0.iter().any(|(n, _)| n.value == name.value);
            match raw
                .servers
                .0
                .iter()
                .position(|(n, _)| n.value == server.value)
            {
                Some(i) if !needs_it(&raw.servers.0[i].1) => b.error(
                    server,
                    format!(
                        "server `{}` doesn't need `{}`, so it can't be in its `order`",
                        server.value, name.value
                    ),
                ),
                Some(i) if order.contains(&i) => {
                    b.error(server, format!("`{}` is listed twice", server.value))
                }
                Some(i) => order.push(i),
                None => b.error(
                    server,
                    format!("`order` lists servers, and `{}` isn't one", server.value),
                ),
            }
        }
        resources.push(Resource {
            name: name.value.clone(),
            capacity: r.capacity.value,
            schedule,
            order,
        });
    }

    let mut servers = Vec::new();
    for (i, (name, s)) in raw.servers.0.iter().enumerate() {
        let kind = b.server_kind(name, s, &types);
        let capacity = match &s.capacity {
            Some(c) if c.value == 0 => {
                b.error(
                    c,
                    format!("server `{}` needs a capacity of at least 1", name.value),
                );
                None
            }
            Some(c) => {
                if let Some(ServerKind::Batch(n)) = kind
                    && c.value % n != 0
                {
                    b.error(
                        c,
                        format!(
                            "server `{}` processes batches of {n}, so its capacity must be a multiple of {n}",
                            name.value
                        ),
                    );
                }
                Some(c.value)
            }
            // A batch server holds one batch by default.
            None => Some(match kind {
                Some(ServerKind::Batch(n)) => n,
                _ => 1,
            }),
        };
        if kind.as_ref().is_some_and(|k| *k != ServerKind::Single)
            && matches!(s.process_time.value, raw::RawPerType::ByType(_))
        {
            b.error(
                &s.process_time,
                format!(
                    "server `{}` processes batches or kits, so it has one process time, not one per part type",
                    name.value
                ),
            );
        }
        let process_time = b.per_type_distribution(name, &s.process_time, &types);
        let schedule = b.schedule_ref(name, &s.schedule, &schedule_names);
        let mut needs = Vec::new();
        let mut needs_ok = true;
        for (resource, units) in &s.needs.0 {
            match objects.get(resource.value.as_str()) {
                Some(Object {
                    kind: Kind::Resource,
                    index,
                }) => {
                    let capacity = raw.resources.0[*index].1.capacity.value;
                    if units.value == 0 {
                        b.error(
                            units,
                            format!(
                                "server `{}` needs at least 1 `{}`; remove it if none",
                                name.value, resource.value
                            ),
                        );
                        needs_ok = false;
                    } else if units.value > capacity {
                        b.error(
                            units,
                            format!(
                                "server `{}` needs {} `{}`, but there are only {capacity}: it could never start",
                                name.value, units.value, resource.value
                            ),
                        );
                        needs_ok = false;
                    }
                    needs.push((*index, units.value));
                }
                _ => {
                    let known: Vec<String> = raw
                        .resources
                        .0
                        .iter()
                        .map(|(n, _)| format!("`{}`", n.value))
                        .collect();
                    b.error(
                        resource,
                        format!(
                            "server `{}` needs `{}`, which isn't listed under `resources` (resources: {})",
                            name.value,
                            resource.value,
                            if known.is_empty() { "none".into() } else { known.join(", ") }
                        ),
                    );
                    needs_ok = false;
                }
            }
        }
        let mut failures = Vec::new();
        let mut failures_ok = true;
        for (j, f) in s.failures.iter().enumerate() {
            b.check_name(&f.name);
            if s.failures[..j].iter().any(|g| g.name.value == f.name.value) {
                b.error(
                    &f.name,
                    format!(
                        "server `{}` has two failures named `{}`",
                        name.value, f.name.value
                    ),
                );
            }
            let uptime = b.distribution(&f.uptime);
            let downtime = b.distribution(&f.downtime);
            match (uptime, downtime) {
                (Some(uptime), Some(downtime)) => failures.push(Failure {
                    name: f.name.value.clone(),
                    uptime,
                    downtime,
                    clock: match f.counts {
                        raw::FailureClock::BusyTime => FailureClock::BusyTime,
                        raw::FailureClock::ClockTime => FailureClock::ClockTime,
                    },
                    on_failure: match f.on_failure {
                        raw::OnFailure::Resume => OnFailure::Resume,
                        raw::OnFailure::Restart => OnFailure::Restart,
                    },
                }),
                _ => failures_ok = false,
            }
        }
        let splits_into = match &s.splits_into {
            None => Some(None),
            Some(split) => match split.value.0.as_slice() {
                [(entity, count)] => {
                    let entity = b.part_type(entity, &entity.value, &types, "`splits_into` names");
                    let count = b.count(
                        &Spanned {
                            value: *count,
                            referenced: split.referenced,
                            defined: split.defined,
                        },
                        "the number of parts in `splits_into`",
                    );
                    match (entity, count) {
                        (Some(e), Some(c)) => Some(Some((e, c))),
                        _ => None,
                    }
                }
                _ => {
                    b.error(
                        split,
                        "`splits_into` names one part type and how many, like `{ carton: 12 }`"
                            .to_string(),
                    );
                    None
                }
            },
        };
        let changeover = match &s.changeover {
            None => Some(None),
            Some(c) => b.changeover(name, c, &types).map(Some),
        };
        let input = endpoints_ok
            .then(|| b.input_rule(name, &s.input, &server_inputs[i], &conveyor_names, &names))
            .flatten();
        let sets = b.sets(&s.sets, &names);
        let output = endpoints_ok
            .then(|| {
                b.output_rule(
                    name,
                    "server",
                    &s.output,
                    &server_outputs[i],
                    &conveyor_names,
                    &types,
                    &names,
                )
            })
            .flatten();
        if let (
            Some(capacity),
            Some(process_time),
            true,
            true,
            Some((inputs, input_rule)),
            Some((outputs, routing)),
            Some(schedule),
            Some(kind),
            Some(splits_into),
            Some(changeover),
            Some(sets),
        ) = (
            capacity,
            process_time,
            failures_ok,
            needs_ok,
            input,
            output,
            schedule,
            kind,
            splits_into,
            changeover,
            sets,
        ) {
            servers.push(Server {
                name: name.value.clone(),
                capacity,
                splits_into,
                changeover,
                process_time,
                needs,
                schedule,
                kind,
                failures,
                inputs,
                input_rule,
                outputs,
                routing,
                sets,
            });
        }
    }

    let mut sinks = Vec::new();
    for (i, (name, _)) in raw.sinks.0.iter().enumerate() {
        if endpoints_ok && sink_inputs[i].is_empty() {
            b.error(
                name,
                format!("sink `{}` has no incoming conveyor or queue", name.value),
            );
        }
        sinks.push(Sink {
            name: name.value.clone(),
        });
    }
    if raw.sources.0.is_empty() {
        b.diagnostics.push(Diagnostic {
            position: None,
            message: "the model has no sources".to_string(),
        });
    }

    let experiment = b.experiment(&raw.experiment);
    // Paced lines: their own namespace (only `lines` names them).
    let mut lines = Vec::new();
    let mut in_a_line: Vec<Option<&str>> = vec![None; raw.servers.0.len()];
    for (name, l) in &raw.lines.0 {
        b.check_name(name);
        let takt = b.positive_quantity(&l.takt, Dimension::Time, "takt");
        let mut stations = Vec::new();
        let mut ok = true;
        for station in &l.stations.value {
            let Some(i) = raw.servers.0.iter().position(|(n, _)| n.value == *station) else {
                b.error(
                    &l.stations,
                    format!("line `{}`: `{station}` isn't a server", name.value),
                );
                ok = false;
                continue;
            };
            if raw.servers.0[i]
                .1
                .capacity
                .as_ref()
                .is_some_and(|c| c.value != 1)
                || raw.servers.0[i].1.batch.is_some()
            {
                b.error(&l.stations, format!("line `{}`: station `{station}` must hold one part at a time (capacity 1, no batches)", name.value));
                ok = false;
            }
            if let Some(other) = in_a_line[i] {
                b.error(
                    &l.stations,
                    format!("`{station}` is already a station of line `{other}`"),
                );
                ok = false;
            }
            in_a_line[i] = Some(name.value.as_str());
            stations.push(i);
        }
        if l.stations.value.len() < 2 {
            b.error(
                &l.stations,
                format!("line `{}` needs at least two stations", name.value),
            );
            ok = false;
        }
        if let (true, Some(takt)) = (ok, takt) {
            lines.push(PacedLine {
                name: name.value.clone(),
                stations,
                takt: SimTime::from_secs_f64(takt),
            });
        }
    }
    let words = match &raw.parts {
        None => Words::default(),
        Some(w) => {
            for word in [&w.one, &w.many] {
                let text = word.value.trim();
                if text.is_empty() || text.len() > 40 || text.contains(['\n', '{', '}']) {
                    b.error(
                        word,
                        "`parts` words are short names, like `one: customer, many: customers`"
                            .to_string(),
                    );
                }
            }
            Words {
                one: w.one.value.trim().to_string(),
                many: w.many.value.trim().to_string(),
            }
        }
    };
    let throughput_per = match &raw.outputs.throughput_per {
        None => ThroughputPer::Hour,
        Some(per) => match per.value.as_str() {
            "hour" => ThroughputPer::Hour,
            "day" => ThroughputPer::Day,
            "week" => ThroughputPer::Week,
            name => match schedule_names.iter().position(|s| *s == name) {
                Some(i) => ThroughputPer::WorkingHour(i),
                None => {
                    b.error(
                        per,
                        format!(
                            "`throughput_per` is `hour`, `day`, `week` or a schedule's name (for per working hour), not `{name}`"
                        ),
                    );
                    ThroughputPer::Hour
                }
            },
        },
    };

    if !b.diagnostics.is_empty() {
        // File order; errors without a position last.
        b.diagnostics.sort_by_key(|d| {
            d.position
                .map_or((true, 0, 0), |p| (false, p.line, p.column))
        });
        return Err(b.diagnostics);
    }

    let mut spec = Spec {
        entities: raw
            .entities
            .0
            .iter()
            .map(|(n, _)| n.value.clone())
            .collect(),
        attributes,
        sources,
        servers,
        conveyors,
        sinks,
        resources,
        schedules,
        vehicles,
        lines,
        experiment: experiment.expect("no diagnostics means the experiment is valid"),
    };
    check_reachability(&spec, &raw)?;
    check_part_types(&spec, &raw)?;
    check_stock(&mut spec, &raw)?;

    Ok(Model {
        name: raw.name.value,
        description: raw.description,
        words,
        spec,
        outputs: Outputs {
            event_log: raw.outputs.event_log,
            summary: raw.outputs.summary,
            throughput_per,
        },
    })
}

/// Starting stock in queues (`starts_with`), and that every restocking
/// source's parts can get to the queue it keeps stocked.
fn check_stock(spec: &mut Spec, raw: &RawModel) -> Result<(), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let mut error = |at: &serde_saphyr::Location, message: String| {
        diagnostics.push(Diagnostic {
            position: Position::from_location(at),
            message,
        })
    };
    let types_at = spec.types_at();
    for (j, (name, q)) in raw.queues.0.iter().enumerate() {
        let i = raw.conveyors.0.len() + j;
        let Some(start) = &q.starts_with else {
            continue;
        };
        let here: Vec<usize> = (0..spec.entities.len())
            .filter(|&e| types_at.conveyors[i][e])
            .collect();
        let found = match &start.value {
            raw::RawStartsWith::Count(n) => match here.as_slice() {
                [e] => Some((*e, *n)),
                _ => {
                    error(
                        &start.referenced,
                        format!(
                            "queue `{}` gets more than one part type; say which it starts with, like `starts_with: {{ {}: {n} }}`",
                            name.value,
                            here.first().map_or("part", |&e| spec.entities[e].as_str())
                        ),
                    );
                    None
                }
            },
            raw::RawStartsWith::Typed(map) => match map.0.as_slice() {
                [(t, n)] => match spec.entities.iter().position(|e| *e == t.value) {
                    Some(e) => Some((e, *n)),
                    None => {
                        error(
                            &t.referenced,
                            format!("`{}` isn't listed under `entities`", t.value),
                        );
                        None
                    }
                },
                _ => {
                    error(
                        &start.referenced,
                        "`starts_with` names one part type and how many".to_string(),
                    );
                    None
                }
            },
        };
        if let Some((e, n)) = found {
            if n > u64::from(spec.conveyors[i].capacity) || n > 1_000_000 {
                error(
                    &start.referenced,
                    format!("queue `{}` can't start with more than it holds", name.value),
                );
            } else {
                spec.conveyors[i].starts_with = Some((e, n as u32));
            }
        }
    }
    for (s, (name, _)) in raw.sources.0.iter().enumerate() {
        let Arrivals::Restock(r) = &spec.sources[s].arrivals else {
            continue;
        };
        // Its parts must be able to reach the queue.
        let mut seen = vec![Node::Source(s)];
        let mut stack = vec![Node::Source(s)];
        let mut reaches = false;
        while let Some(n) = stack.pop() {
            for (c, conveyor) in spec
                .conveyors
                .iter()
                .enumerate()
                .filter(|(_, c)| c.from == n)
            {
                if c == r.queue {
                    reaches = true;
                }
                if !seen.contains(&conveyor.to) {
                    seen.push(conveyor.to);
                    stack.push(conveyor.to);
                }
            }
        }
        if !reaches {
            error(
                &name.referenced,
                format!(
                    "source `{}` restocks `{}`, but its parts can't get there",
                    name.value, spec.conveyors[r.queue].name
                ),
            );
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

/// Every server must be fed by a source, and every part must be able to
/// reach a sink: otherwise parts circulate or wait forever.
fn check_reachability(spec: &Spec, raw: &RawModel) -> Result<(), Vec<Diagnostic>> {
    // Forward from the sources.
    let mut fed = vec![false; spec.servers.len()];
    let mut stack: Vec<Node> = (0..spec.sources.len()).map(Node::Source).collect();
    while let Some(node) = stack.pop() {
        for c in &spec.conveyors {
            if c.from == node
                && let Node::Server(s) = c.to
                && !fed[s]
            {
                fed[s] = true;
                stack.push(c.to);
            }
        }
    }
    // Backward from the sinks.
    let drains = |node: Node| -> bool {
        let mut seen = vec![node];
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            for c in spec.conveyors.iter().filter(|c| c.from == n) {
                if matches!(c.to, Node::Sink(_)) {
                    return true;
                }
                if !seen.contains(&c.to) {
                    seen.push(c.to);
                    stack.push(c.to);
                }
            }
        }
        false
    };

    let mut diagnostics = Vec::new();
    for (s, (name, _)) in raw.servers.0.iter().enumerate() {
        if !fed[s] {
            diagnostics.push(Diagnostic {
                position: Position::from_location(&name.referenced),
                message: format!("server `{}` can't be reached from any source", name.value),
            });
        } else if !drains(Node::Server(s)) {
            diagnostics.push(Diagnostic {
                position: Position::from_location(&name.referenced),
                message: format!(
                    "parts at server `{}` can never reach a sink: every path from it loops back",
                    name.value
                ),
            });
        }
    }
    for (s, (name, _)) in raw.sources.0.iter().enumerate() {
        if !drains(Node::Source(s)) {
            diagnostics.push(Diagnostic {
                position: Position::from_location(&name.referenced),
                message: format!("parts from source `{}` can never reach a sink", name.value),
            });
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

/// Which part types can reach each object, following `by_type` routes:
/// every server must have a process time, and every `by_type` rule a
/// route, for each of them.
fn check_part_types(spec: &Spec, raw: &RawModel) -> Result<(), Vec<Diagnostic>> {
    let n = spec.entities.len();
    let reach = spec.types_at();
    let (at_server, at_source) = (&reach.servers, &reach.sources);
    let mut diagnostics = Vec::new();
    let mut report = |name: &Spanned<String>, message: String| {
        diagnostics.push(Diagnostic {
            position: Position::from_location(&name.referenced),
            message,
        })
    };
    let missing = |arrives: &[bool], has: &dyn Fn(usize) -> bool| -> Vec<String> {
        (0..n)
            .filter(|&e| arrives[e] && !has(e))
            .map(|e| format!("`{}`", spec.entities[e]))
            .collect()
    };
    for (i, (name, _)) in raw.servers.0.iter().enumerate() {
        let server = &spec.servers[i];
        if let ServerKind::Assembly { recipe, makes, .. } = &server.kind {
            let foreign = missing(&at_server[i], &|e| recipe.iter().any(|&(r, _)| r == e));
            if !foreign.is_empty() {
                report(
                    name,
                    format!(
                        "server `{}` gets parts of type {}, which aren't in what it assembles: they'd wait forever",
                        name.value,
                        foreign.join(", ")
                    ),
                );
            }
            let never: Vec<String> = recipe
                .iter()
                .filter(|&&(e, _)| !at_server[i][e])
                .map(|&(e, _)| format!("`{}`", spec.entities[e]))
                .collect();
            if !never.is_empty() {
                report(
                    name,
                    format!(
                        "server `{}` assembles {}, but none can reach it: no kit would ever be complete",
                        name.value,
                        never.join(", ")
                    ),
                );
            }
            // Routing after an assembly only ever sees what it makes.
            if let Routing::ByType(routes) = &server.routing
                && routes[*makes].is_none()
            {
                report(
                    name,
                    format!(
                        "server `{}` makes `{}`, but its `by_type` rule has no route for it",
                        name.value, spec.entities[*makes]
                    ),
                );
            }
            continue;
        }
        if let PerType::ByType(times) = &server.process_time {
            let gaps = missing(&at_server[i], &|e| times[e].is_some());
            if !gaps.is_empty() {
                report(
                    name,
                    format!(
                        "server `{}` gets parts of type {}, but `process_time` has no time for them",
                        name.value,
                        gaps.join(", ")
                    ),
                );
            }
        }
        if let Routing::ByType(routes) = &server.routing {
            let gaps = missing(&at_server[i], &|e| routes[e].is_some());
            if !gaps.is_empty() {
                report(
                    name,
                    format!(
                        "server `{}` gets parts of type {}, but its `by_type` rule has no route for them",
                        name.value,
                        gaps.join(", ")
                    ),
                );
            }
        }
    }
    for (i, (name, _)) in raw.sources.0.iter().enumerate() {
        if let Routing::ByType(routes) = &spec.sources[i].routing {
            let gaps = missing(&at_source[i], &|e| routes[e].is_some());
            if !gaps.is_empty() {
                report(
                    name,
                    format!(
                        "source `{}` creates parts of type {}, but its `by_type` rule has no route for them",
                        name.value,
                        gaps.join(", ")
                    ),
                );
            }
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

/// The model's part types, for checking names given per type.
struct Types<'a> {
    index: &'a HashMap<&'a str, usize>,
    names: &'a [&'a str],
}

impl Types<'_> {
    fn list(&self) -> String {
        if self.names.is_empty() {
            return "none".into();
        }
        self.names
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Source,
    Server,
    Conveyor,
    Queue,
    Transport,
    Fleet,
    Sink,
    Resource,
}

impl Kind {
    fn describe(self) -> &'static str {
        match self {
            Kind::Source => "source",
            Kind::Server => "server",
            Kind::Conveyor => "conveyor",
            Kind::Queue => "queue",
            Kind::Transport => "transport",
            Kind::Fleet => "fleet of vehicles",
            Kind::Sink => "sink",
            Kind::Resource => "resource",
        }
    }
}

#[derive(Clone, Copy)]
struct Object {
    kind: Kind,
    index: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum End {
    From,
    To,
}

#[derive(Default)]
struct Builder {
    defaults: DefaultUnits,
    diagnostics: Vec<Diagnostic>,
}

impl Builder {
    fn error<T>(&mut self, at: &Spanned<T>, message: String) {
        self.diagnostics.push(Diagnostic {
            position: Position::from_location(&at.referenced),
            message,
        });
    }

    /// Names end up in event logs, CSV columns and command lines, so keep them simple.
    fn check_name(&mut self, name: &Spanned<String>) {
        let mut chars = name.value.chars();
        let valid = chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !valid {
            self.error(
                name,
                format!(
                    "`{}` is not a valid name: use letters, digits, `_` and `-`, starting with a letter or `_`",
                    name.value
                ),
            );
        }
    }

    fn default_unit(
        &mut self,
        unit: &Option<Spanned<String>>,
        dimension: Dimension,
    ) -> Option<String> {
        let unit = unit.as_ref()?;
        match DefaultUnits::check(&unit.value, dimension) {
            Ok(()) => Some(unit.value.clone()),
            Err(message) => {
                self.error(unit, message);
                None
            }
        }
    }

    fn quantity(&mut self, value: &Spanned<Scalar>, dimension: Dimension) -> Option<f64> {
        let result = match &value.value {
            Scalar::Number(n) => convert_number(*n, dimension, &self.defaults),
            Scalar::Text(t) => parse_quantity(t, dimension, &self.defaults),
        };
        result.map_err(|message| self.error(value, message)).ok()
    }

    fn positive_quantity(
        &mut self,
        value: &Spanned<Scalar>,
        dimension: Dimension,
        what: &str,
    ) -> Option<f64> {
        let quantity = self.quantity(value, dimension)?;
        if quantity <= 0.0 {
            self.error(value, format!("{what} must be greater than zero"));
            return None;
        }
        Some(quantity)
    }

    /// `arrivals_per_hour`: periods of the day (`"12:00-14:00"`) with a rate
    /// each, as (start, end, parts per second) in seconds from midnight. A
    /// period that runs past midnight (`"22:00-02:00"`) is split in two.
    fn arrival_profile(
        &mut self,
        source: &Spanned<String>,
        raw: &Spanned<raw::OrderedMap<f64>>,
    ) -> Option<Vec<(f64, f64, f64)>> {
        let clock = |text: &str, end: bool| -> Option<f64> {
            let (h, m) = text.trim().split_once(':')?;
            let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
            let valid = m <= 59 && h < 24 || (end && h == 24 && m == 0);
            valid.then_some(f64::from(h * 3600 + m * 60))
        };
        let mut ok = true;
        let mut periods = Vec::new();
        for (period, rate) in &raw.value.0 {
            let rate = *rate;
            let parsed = period
                .value
                .split_once('-')
                .and_then(|(a, b)| Some((clock(a, false)?, clock(b, true)?)));
            let Some((start, end)) = parsed else {
                self.error(
                    period,
                    format!(
                        "`{}` isn't a time of day: write it like `12:00-14:00`",
                        period.value
                    ),
                );
                ok = false;
                continue;
            };
            if !(rate.is_finite() && rate >= 0.0) {
                self.error(
                    period,
                    format!(
                        "the rate for `{}` must be zero or more arrivals per hour",
                        period.value
                    ),
                );
                ok = false;
                continue;
            }
            let per_second = rate / 3600.0;
            if start < end {
                periods.push((start, end, per_second, period));
            } else if start > end {
                periods.push((start, 24.0 * 3600.0, per_second, period));
                periods.push((0.0, end, per_second, period));
            } else {
                self.error(
                    period,
                    format!("`{}` starts and ends at the same time", period.value),
                );
                ok = false;
            }
        }
        periods.sort_by(|a, b| a.0.total_cmp(&b.0));
        for w in periods.windows(2) {
            if w[1].0 < w[0].1 {
                self.error(
                    w[1].3,
                    format!("`{}` overlaps `{}`", w[1].3.value, w[0].3.value),
                );
                ok = false;
            }
        }
        if ok && !periods.iter().any(|p| p.2 > 0.0) {
            self.error(
                raw,
                format!(
                    "source `{}` never has any arrivals: every rate is zero",
                    source.value
                ),
            );
            ok = false;
        }
        ok.then(|| periods.into_iter().map(|(s, e, r, _)| (s, e, r)).collect())
    }

    /// The index of a declared attribute, or an error at `name`.
    fn attribute(&mut self, name: &Spanned<String>, names: &Names) -> Option<usize> {
        let found = names.attributes.iter().position(|(n, _)| *n == name.value);
        if found.is_none() {
            let known: Vec<String> = names
                .attributes
                .iter()
                .map(|(n, _)| format!("`{n}`"))
                .collect();
            self.error(
                name,
                format!(
                    "`{}` isn't listed under `attributes` (attributes: {})",
                    name.value,
                    if known.is_empty() {
                        "none".into()
                    } else {
                        known.join(", ")
                    }
                ),
            );
        }
        found
    }

    /// A queue's `order`.
    fn queue_order(
        &mut self,
        order: &Spanned<raw::RawQueueOrder>,
        names: &Names,
    ) -> Option<QueueOrder> {
        let at = |value: &String| Spanned {
            value: value.clone(),
            referenced: order.referenced,
            defined: order.defined,
        };
        match &order.value {
            raw::RawQueueOrder::FirstIn => Some(QueueOrder::FirstIn),
            raw::RawQueueOrder::LastIn => Some(QueueOrder::LastIn),
            raw::RawQueueOrder::Lowest(a) => self.attribute(&at(a), names).map(QueueOrder::Lowest),
            raw::RawQueueOrder::Highest(a) => {
                self.attribute(&at(a), names).map(QueueOrder::Highest)
            }
            raw::RawQueueOrder::ByType(types) => {
                // Types not listed come after those listed.
                let mut rank = vec![types.len(); names.entities.len()];
                let mut ok = true;
                for (i, t) in types.iter().enumerate() {
                    match names.entities.iter().position(|e| e == t) {
                        Some(e) if rank[e] < types.len() => {
                            self.error(order, format!("`{t}` is listed twice"));
                            ok = false;
                        }
                        Some(e) => rank[e] = i,
                        None => {
                            self.error(
                                order,
                                format!(
                                    "`{t}` isn't a part type (part types: {})",
                                    names.entities.join(", ")
                                ),
                            );
                            ok = false;
                        }
                    }
                }
                ok.then_some(QueueOrder::ByType(rank))
            }
        }
    }

    /// `sets`: attribute -> formula, each giving what its attribute holds.
    fn sets(
        &mut self,
        raw: &raw::OrderedMap<Spanned<Scalar>>,
        names: &Names,
    ) -> Option<Vec<(usize, Expr)>> {
        let mut sets = Vec::new();
        let mut ok = true;
        for (attribute, value) in &raw.0 {
            let Some(i) = names
                .attributes
                .iter()
                .position(|(n, _)| *n == attribute.value)
            else {
                let known: Vec<String> = names
                    .attributes
                    .iter()
                    .map(|(n, _)| format!("`{n}`"))
                    .collect();
                self.error(
                    attribute,
                    format!(
                        "`{}` isn't listed under `attributes` (attributes: {})",
                        attribute.value,
                        if known.is_empty() {
                            "none".into()
                        } else {
                            known.join(", ")
                        }
                    ),
                );
                ok = false;
                continue;
            };
            let text = match &value.value {
                Scalar::Number(n) => n.to_string(),
                Scalar::Text(t) => t.clone(),
            };
            match formula::parse(&text, names) {
                Ok((expr, kind)) if kind == names.attributes[i].1 => sets.push((i, expr)),
                Ok((_, kind)) => {
                    self.error(
                        value,
                        format!(
                            "`{}` holds {}, but `{}` is {}",
                            attribute.value,
                            names.attributes[i].1.describe(),
                            text.trim(),
                            kind.describe()
                        ),
                    );
                    ok = false;
                }
                Err(message) => {
                    self.error(value, message);
                    ok = false;
                }
            }
        }
        ok.then_some(sets)
    }

    fn distribution(&mut self, value: &Spanned<Scalar>) -> Option<Distribution> {
        self.distribution_at(value, &value.value, "")
    }

    /// A distribution, with errors reported at `at` (prefixed with `what`).
    fn distribution_at<T>(
        &mut self,
        at: &Spanned<T>,
        value: &Scalar,
        what: &str,
    ) -> Option<Distribution> {
        let result = match value {
            Scalar::Number(n) => {
                convert_number(*n, Dimension::Time, &self.defaults).and_then(constant)
            }
            Scalar::Text(t) => parse_time_distribution(t, &self.defaults),
        };
        result
            .map_err(|message| self.error(at, format!("{what}{message}")))
            .ok()
    }

    /// What a server does with its parts: one at a time, batches or kits.
    fn server_kind(
        &mut self,
        name: &Spanned<String>,
        s: &raw::RawServer,
        types: &Types,
    ) -> Option<ServerKind> {
        match (&s.batch, &s.assembles, &s.makes) {
            (Some(batch), Some(_), _) => {
                self.error(
                    batch,
                    format!("server `{}` can batch or assemble, not both", name.value),
                );
                None
            }
            (Some(batch), None, makes) => {
                if let Some(makes) = makes {
                    self.error(makes, "`makes` goes with `assembles`".to_string());
                    return None;
                }
                if batch.value == 0 {
                    self.error(
                        batch,
                        format!("server `{}` needs a batch of at least 1", name.value),
                    );
                    return None;
                }
                Some(ServerKind::Batch(batch.value))
            }
            (None, Some(recipe), makes) => {
                let Some(makes) = makes else {
                    self.error(
                        name,
                        format!(
                            "server `{}` assembles parts; add `makes:` with the part type it makes",
                            name.value
                        ),
                    );
                    return None;
                };
                let what = format!("server `{}` makes", name.value);
                let made = self.part_type(makes, &makes.value, types, &what);
                let mut out = Vec::new();
                let mut ok = true;
                for (part_type, count) in &recipe.0 {
                    let what = format!("server `{}` assembles", name.value);
                    let e = self.part_type(part_type, &part_type.value, types, &what);
                    if count.value == 0 {
                        self.error(
                            count,
                            "a kit needs at least 1 of each component".to_string(),
                        );
                        ok = false;
                    }
                    match e {
                        Some(e) => out.push((e, count.value)),
                        None => ok = false,
                    }
                }
                if recipe.0.is_empty() {
                    self.error(name, format!("server `{}` assembles nothing", name.value));
                    ok = false;
                }
                let clock_of = match &s.keeps_clock_of {
                    None => None,
                    Some(t) => {
                        let what = format!("server `{}` keeps the clock of", name.value);
                        match self.part_type(t, &t.value, types, &what) {
                            Some(e) if out.iter().any(|&(c, _)| c == e) => Some(e),
                            Some(_) => {
                                self.error(t, format!("`keeps_clock_of` names a part type in the kit, and `{}` isn't", t.value));
                                ok = false;
                                None
                            }
                            None => {
                                ok = false;
                                None
                            }
                        }
                    }
                };
                let makes = made?;
                ok.then_some(ServerKind::Assembly {
                    recipe: out,
                    makes,
                    clock_of,
                })
            }
            (None, None, Some(makes)) => {
                self.error(makes, "`makes` goes with `assembles`".to_string());
                None
            }
            (None, None, None) => {
                if let Some(t) = &s.keeps_clock_of {
                    self.error(t, "`keeps_clock_of` goes with `assembles`".to_string());
                    return None;
                }
                Some(ServerKind::Single)
            }
        }
    }

    /// An optional `schedule:` reference: `Some(None)` when absent,
    /// `Some(Some(i))` when known, `None` (an error) when unknown.
    fn schedule_ref(
        &mut self,
        object: &Spanned<String>,
        schedule: &Option<Spanned<String>>,
        names: &[&str],
    ) -> Option<Option<usize>> {
        let Some(schedule) = schedule else {
            return Some(None);
        };
        match names.iter().position(|n| *n == schedule.value) {
            Some(i) => Some(Some(i)),
            None => {
                let known = if names.is_empty() {
                    "none".to_string()
                } else {
                    names
                        .iter()
                        .map(|n| format!("`{n}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                self.error(
                    schedule,
                    format!(
                        "`{}` follows schedule `{}`, which isn't listed under `schedules` (schedules: {known})",
                        object.value, schedule.value
                    ),
                );
                None
            }
        }
    }

    /// A schedule's working periods within the week (see [`Schedule`]).
    fn schedule(
        &mut self,
        name: &Spanned<String>,
        raw: &raw::RawSchedule,
    ) -> Option<Vec<(SimTime, SimTime)>> {
        const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
        const DAY: i64 = 24 * 3600 * 1_000_000;
        let week = Schedule::WEEK.as_micros();
        let mut ok = true;
        let days: Vec<usize> = match &raw.days {
            None => (0..7).collect(),
            Some(days) => {
                let mut out = Vec::new();
                for d in &days.value {
                    match DAYS.iter().position(|x| x.eq_ignore_ascii_case(d.trim())) {
                        Some(i) if out.contains(&i) => {
                            self.error(days, format!("`{d}` is listed twice"));
                            ok = false;
                        }
                        Some(i) => out.push(i),
                        None => {
                            self.error(
                                days,
                                format!("`{d}` isn't a day: use mon, tue, wed, thu, fri, sat, sun"),
                            );
                            ok = false;
                        }
                    }
                }
                if days.value.is_empty() {
                    self.error(days, format!("schedule `{}` has no days", name.value));
                    ok = false;
                }
                out
            }
        };
        if raw.shifts.value.is_empty() {
            self.error(
                &raw.shifts,
                format!("schedule `{}` has no shifts", name.value),
            );
            return None;
        }
        let clock = |text: &str, end: bool| -> Option<i64> {
            let (h, m) = text.trim().split_once(':')?;
            let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
            let valid = (0..=59).contains(&m) && (0..24).contains(&h) || (end && h == 24 && m == 0);
            valid.then_some((h * 60 + m) * 60 * 1_000_000)
        };
        let mut periods: Vec<(i64, i64)> = Vec::new();
        for shift in &raw.shifts.value {
            let parsed = shift
                .split_once('-')
                .and_then(|(a, b)| Some((clock(a, false)?, clock(b, true)?)));
            let Some((start, end)) = parsed else {
                self.error(
                    &raw.shifts,
                    format!("`{shift}` isn't a shift: write it like `06:00-14:00`"),
                );
                ok = false;
                continue;
            };
            if start == end {
                self.error(
                    &raw.shifts,
                    format!(
                        "`{shift}` starts and ends at the same time; `00:00-24:00` is a whole day"
                    ),
                );
                ok = false;
                continue;
            }
            for &d in &days {
                let s = d as i64 * DAY + start;
                // Ending at or before its start: the shift ends the next day.
                let e = d as i64 * DAY + end + if end <= start { DAY } else { 0 };
                if e > week {
                    periods.push((s, week));
                    periods.push((0, e - week));
                } else {
                    periods.push((s, e));
                }
            }
        }
        if !ok {
            return None;
        }
        // Sorted, overlapping or touching periods merged.
        periods.sort();
        let mut merged: Vec<(i64, i64)> = Vec::new();
        for (s, e) in periods {
            match merged.last_mut() {
                Some(last) if s <= last.1 => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }
        Some(
            merged
                .into_iter()
                .map(|(s, e)| (SimTime::from_micros(s), SimTime::from_micros(e)))
                .collect(),
        )
    }

    /// A restocking source: the queue it keeps stocked, when and how much
    /// it orders, and the lead time.
    fn restock(
        &mut self,
        name: &Spanned<String>,
        s: &raw::RawSource,
        raw: &RawModel,
    ) -> Option<Restock> {
        let what = format!(
            "source `{}` restocks a queue, so it can't also have",
            name.value
        );
        for (field, present) in [
            (
                "interarrival",
                s.interarrival.as_ref().map(|v| (v.referenced, v.defined)),
            ),
            (
                "arrivals_per_hour",
                s.arrivals_per_hour
                    .as_ref()
                    .map(|v| (v.referenced, v.defined)),
            ),
            ("group", s.group.as_ref().map(|v| (v.referenced, v.defined))),
        ] {
            if let Some((referenced, defined)) = present {
                self.error(
                    &Spanned {
                        value: (),
                        referenced,
                        defined,
                    },
                    format!("{what} `{field}`: its orders bring its parts"),
                );
            }
        }
        let queue_name = s.restocks.as_ref().expect("caller checked");
        let queue = match raw
            .queues
            .0
            .iter()
            .position(|(n, _)| n.value == queue_name.value)
        {
            Some(j) => Some(raw.conveyors.0.len() + j),
            None => {
                self.error(
                    queue_name,
                    format!(
                        "`restocks` names a queue, and there's no queue `{}`",
                        queue_name.value
                    ),
                );
                None
            }
        };
        let reorder_point = match &s.reorder_point {
            Some(p) => Some(p.value),
            None => {
                self.error(
                    name,
                    format!(
                        "source `{}` needs a `reorder_point`: order when the stock falls to this",
                        name.value
                    ),
                );
                None
            }
        };
        let quantity = match (&s.order, &s.order_up_to) {
            (Some(q), None) if q.value >= 1 => Some(OrderQuantity::Fixed(q.value)),
            (Some(q), None) => {
                self.error(q, "`order` is at least 1 part".to_string());
                None
            }
            (None, Some(level)) => match reorder_point {
                Some(p) if level.value <= p => {
                    self.error(
                        level,
                        format!("`order_up_to` must be above the reorder point ({p})"),
                    );
                    None
                }
                _ => Some(OrderQuantity::UpTo(level.value)),
            },
            (Some(q), Some(_)) => {
                self.error(
                    q,
                    "give `order` (a fixed amount) or `order_up_to` (a level), not both"
                        .to_string(),
                );
                None
            }
            (None, None) => {
                self.error(name, format!("source `{}` needs `order` (parts per order) or `order_up_to` (a stock level)", name.value));
                None
            }
        };
        let lead_time = match &s.lead_time {
            Some(t) => self.distribution(t),
            None => {
                self.error(
                    name,
                    format!(
                        "source `{}` needs a `lead_time`: how long an order takes to come",
                        name.value
                    ),
                );
                None
            }
        };
        let review_every = match &s.review_every {
            None => Some(None),
            Some(t) => self
                .positive_quantity(t, Dimension::Time, "review_every")
                .map(|secs| Some(SimTime::from_secs_f64(secs))),
        };
        Some(Restock {
            queue: queue?,
            reorder_point: reorder_point?,
            quantity: quantity?,
            lead_time: lead_time?,
            review_every: review_every?,
        })
    }

    /// A whole number of parts (at least 1), fixed or a uniform range.
    fn count(&mut self, at: &Spanned<raw::RawCount>, what: &str) -> Option<Count> {
        const MOST: u64 = 100_000;
        let ok = |n: u64| (1..=MOST).contains(&n);
        match at.value {
            raw::RawCount::Fixed(n) if ok(n) => Some(Count::Fixed(n as u32)),
            raw::RawCount::Range { min, max } if ok(min) && ok(max) && min <= max => {
                Some(Count::Uniform {
                    min: min as u32,
                    max: max as u32,
                })
            }
            _ => {
                self.error(
                    at,
                    format!("{what} is a whole number from 1 to {MOST}, or `{{ min, max }}` with min ≤ max"),
                );
                None
            }
        }
    }

    /// A part type by name, or an error (at `at`) naming the known ones.
    fn part_type<T>(
        &mut self,
        at: &Spanned<T>,
        name: &str,
        types: &Types,
        what: &str,
    ) -> Option<usize> {
        match types.index.get(name) {
            Some(&e) => Some(e),
            None => {
                self.error(
                    at,
                    format!(
                        "{what} `{name}`, which isn't listed under `entities` (part types: {})",
                        types.list()
                    ),
                );
                None
            }
        }
    }

    /// What a source creates: (type, share) pairs, shares summing to 1.
    fn creates(
        &mut self,
        source: &Spanned<String>,
        creates: &Spanned<raw::RawCreates>,
        types: &Types,
    ) -> Option<Vec<(usize, f64)>> {
        let what = format!("source `{}` creates", source.value);
        match &creates.value {
            raw::RawCreates::One(name) => {
                Some(vec![(self.part_type(creates, name, types, &what)?, 1.0)])
            }
            raw::RawCreates::Mix(mix) => {
                let mut out = Vec::new();
                let mut ok = true;
                for (name, share) in mix {
                    match self.part_type(creates, name, types, &what) {
                        Some(e) if out.iter().any(|&(f, _)| f == e) => {
                            self.error(creates, format!("`{name}` is listed twice"));
                            ok = false;
                        }
                        Some(e) => out.push((e, *share)),
                        None => ok = false,
                    }
                    if !(share.is_finite() && *share > 0.0) {
                        self.error(
                            creates,
                            format!("the share of `{name}` must be greater than zero"),
                        );
                        ok = false;
                    }
                }
                if mix.is_empty() {
                    self.error(
                        creates,
                        format!("source `{}` creates nothing", source.value),
                    );
                    return None;
                }
                let sum: f64 = out.iter().map(|(_, s)| s).sum();
                if ok && (sum - 1.0).abs() > 1e-6 {
                    self.error(
                        creates,
                        format!(
                            "the shares of the part types source `{}` creates add up to {sum}, not 1",
                            source.value
                        ),
                    );
                    ok = false;
                }
                ok.then_some(out)
            }
        }
    }

    /// A time distribution, the same for every part type or given per type.
    /// A changeover: one time for any change of part type, or a time per
    /// change written `from->to` (other changes take none).
    fn changeover(
        &mut self,
        server: &Spanned<String>,
        value: &Spanned<raw::RawPerType>,
        types: &Types,
    ) -> Option<Changeover> {
        match &value.value {
            raw::RawPerType::Same(v) => Some(Changeover::Any(self.distribution_at(value, v, "")?)),
            raw::RawPerType::ByType(map) => {
                let n = types.names.len();
                let mut pairs = vec![vec![None; n]; n];
                let mut ok = true;
                let what = format!("server `{}` has a changeover", server.value);
                for (key, v) in map {
                    let Some((from, to)) = key.split_once("->") else {
                        self.error(value, format!("`{key}` isn't a change of part type: write it like `standard->premium`"));
                        ok = false;
                        continue;
                    };
                    let from = self.part_type(value, from.trim(), types, &format!("{what} from"));
                    let to = self.part_type(value, to.trim(), types, &format!("{what} to"));
                    let d = self.distribution_at(value, v, &format!("`{key}`: "));
                    match (from, to, d) {
                        (Some(f), Some(t), Some(d)) if f != t => pairs[f][t] = Some(d),
                        (Some(_), Some(_), Some(_)) => {
                            self.error(
                                value,
                                format!("`{key}` isn't a change: it's the same part type"),
                            );
                            ok = false;
                        }
                        _ => ok = false,
                    }
                }
                ok.then_some(Changeover::Pairs(pairs))
            }
        }
    }

    fn per_type_distribution(
        &mut self,
        server: &Spanned<String>,
        value: &Spanned<raw::RawPerType>,
        types: &Types,
    ) -> Option<PerType<Distribution>> {
        match &value.value {
            raw::RawPerType::Same(v) => Some(PerType::Same(self.distribution_at(value, v, "")?)),
            raw::RawPerType::ByType(map) => {
                let mut out: Vec<Option<Distribution>> = vec![None; types.names.len()];
                let mut ok = true;
                let what = format!("server `{}` has a process time for", server.value);
                for (name, v) in map {
                    let e = self.part_type(value, name, types, &what);
                    let d = self.distribution_at(value, v, &format!("`{name}`: "));
                    match (e, d) {
                        (Some(e), Some(d)) => out[e] = Some(d),
                        _ => ok = false,
                    }
                }
                ok.then_some(PerType::ByType(out))
            }
        }
    }

    fn duration(
        &mut self,
        value: &Spanned<Scalar>,
        allow_zero: bool,
        what: &str,
    ) -> Option<SimTime> {
        let seconds = self.quantity(value, Dimension::Time)?;
        if seconds < 0.0 || (!allow_zero && seconds == 0.0) {
            let bound = if allow_zero {
                "negative"
            } else {
                "zero or negative"
            };
            self.error(value, format!("{what} can't be {bound}"));
            return None;
        }
        if seconds * 1e6 >= i64::MAX as f64 / 2.0 {
            self.error(value, format!("{what} is too long"));
            return None;
        }
        Some(SimTime::from_secs_f64(seconds))
    }

    fn endpoint(
        &mut self,
        objects: &HashMap<&str, Object>,
        what: &str,
        conveyor: &Spanned<String>,
        target: &Spanned<String>,
        end: End,
    ) -> Option<Node> {
        let (word, verb) = match end {
            End::From => ("from", "starts at"),
            End::To => ("to", "goes to"),
        };
        let Some(object) = objects.get(target.value.as_str()) else {
            self.error(
                target,
                format!(
                    "{what} `{}` {verb} `{}`, which doesn't exist",
                    conveyor.value, target.value
                ),
            );
            return None;
        };
        let node = match (object.kind, end) {
            (Kind::Source, End::From) => Node::Source(object.index),
            (Kind::Server, _) => Node::Server(object.index),
            (Kind::Sink, End::To) => Node::Sink(object.index),
            (kind, _) => {
                let reason = match kind {
                    Kind::Source => "nothing can go into a source",
                    Kind::Sink => "nothing comes out of a sink",
                    Kind::Resource => "it's a resource, which servers use, not a place parts go",
                    Kind::Fleet => "it's vehicles, which carry parts along transports",
                    _ => "conveyors and queues connect objects, not other conveyors or queues",
                };
                self.error(
                    target,
                    format!(
                        "{what} `{}` can't have `{word}: {}`: {reason}",
                        conveyor.value, target.value
                    ),
                );
                return None;
            }
        };
        Some(node)
    }

    /// The outgoing conveyors of a source or server and how parts choose
    /// among them. A rule is required with two or more.
    // Each argument is a different lookup the rules need; bundling them would only hide that.
    #[allow(clippy::too_many_arguments)]
    fn output_rule(
        &mut self,
        name: &Spanned<String>,
        kind: &str,
        rule: &Option<Spanned<raw::RawOutputRule>>,
        outputs: &[usize],
        conveyor_names: &[&str],
        types: &Types,
        names: &Names,
    ) -> Option<(Vec<usize>, Routing)> {
        let list = |items: &[usize]| {
            items
                .iter()
                .map(|&c| format!("`{}`", conveyor_names[c]))
                .collect::<Vec<_>>()
                .join(", ")
        };
        if outputs.is_empty() {
            self.error(
                name,
                format!("{kind} `{}` has no outgoing conveyor", name.value),
            );
            return None;
        }
        let Some(rule) = rule else {
            if outputs.len() == 1 {
                return Some((outputs.to_vec(), Routing::Single));
            }
            self.error(
                name,
                format!(
                    "{kind} `{}` has {} outgoing conveyors ({}); add `output: {{ rule: probability | round_robin | first_available | shortest_queue | by_type | by_condition }}` to say how parts choose",
                    name.value,
                    outputs.len(),
                    list(outputs)
                ),
            );
            return None;
        };
        let rule = &rule.value;
        let routes_apply = matches!(
            rule.rule.value,
            raw::OutputRuleKind::ByType | raw::OutputRuleKind::ByCondition
        );
        if rule.routes.is_some() && !routes_apply {
            self.error(
                &rule.rule,
                "`routes` only apply to `rule: by_type` and `rule: by_condition`".to_string(),
            );
            return None;
        }
        if rule.weights.is_some() && rule.rule.value != raw::OutputRuleKind::Probability {
            self.error(
                &rule.rule,
                "`weights` only apply to `rule: probability`".to_string(),
            );
            return None;
        }
        let outgoing = |b: &mut Self, conveyor: &Spanned<String>| {
            let c = outputs
                .iter()
                .copied()
                .find(|&c| conveyor_names[c] == conveyor.value);
            if c.is_none() {
                b.error(
                    conveyor,
                    format!(
                        "`{}` isn't an outgoing conveyor of `{}` (those are {})",
                        conveyor.value,
                        name.value,
                        list(outputs)
                    ),
                );
            }
            c
        };
        let routing = match rule.rule.value {
            raw::OutputRuleKind::ByCondition => {
                let routes = match rule.routes.as_ref().map(|r| (&r.value, r)) {
                    Some((raw::RawRoutes::Conditions(routes), _)) if !routes.is_empty() => routes,
                    Some((_, at)) => {
                        self.error(
                            at,
                            "`rule: by_condition` takes a list of routes: `- { when: \"rework >= 2\", to: c_scrap }`, ending with `- { to: <conveyor> }` for every other part".to_string(),
                        );
                        return None;
                    }
                    None => {
                        self.error(
                            &rule.rule,
                            format!(
                                "`rule: by_condition` needs `routes`: a list of `{{ when: <condition>, to: <conveyor> }}` (one of {}), ending with `{{ to: <conveyor> }}` for every other part",
                                list(outputs)
                            ),
                        );
                        return None;
                    }
                };
                let mut found = Vec::new();
                let mut ok = true;
                for (i, route) in routes.iter().enumerate() {
                    let last = i + 1 == routes.len();
                    let to = outgoing(self, &route.to);
                    let when = match (&route.when, last) {
                        (None, true) => Some(None),
                        (None, false) => {
                            self.error(
                                &route.to,
                                "only the last route goes without `when`: a route without one takes every part, so those after it would never be used".to_string(),
                            );
                            None
                        }
                        (Some(when), true) => {
                            self.error(
                                when,
                                "the last route has no `when`: it takes every part the conditions before it don't, so none is left to chance".to_string(),
                            );
                            None
                        }
                        (Some(when), false) => {
                            let text = match &when.value {
                                Scalar::Number(n) => n.to_string(),
                                Scalar::Text(t) => t.clone(),
                            };
                            match formula::parse(&text, names) {
                                Ok((expr, FormulaKind::Truth)) => Some(Some(expr)),
                                Ok((_, kind)) => {
                                    self.error(
                                        when,
                                        format!(
                                            "`when` is a condition, like `rework >= 2`, but `{}` is {}",
                                            text.trim(),
                                            kind.describe()
                                        ),
                                    );
                                    None
                                }
                                Err(message) => {
                                    self.error(when, message);
                                    None
                                }
                            }
                        }
                    };
                    match (when, to) {
                        (Some(when), Some(to)) => found.push((when, to)),
                        _ => ok = false,
                    }
                }
                if !ok {
                    return None;
                }
                Routing::ByCondition(found)
            }
            raw::OutputRuleKind::ByType => {
                let routes = match rule.routes.as_ref().map(|r| (&r.value, r)) {
                    Some((raw::RawRoutes::ByType(routes), _)) => Some(routes),
                    Some((_, at)) => {
                        self.error(
                            at,
                            "`rule: by_type` takes `routes` as part type -> conveyor, like `{ frame: c_paint }`".to_string(),
                        );
                        return None;
                    }
                    None => None,
                };
                let Some(routes) = routes else {
                    self.error(
                        &rule.rule,
                        format!(
                            "`rule: by_type` needs `routes`: part type -> conveyor (one of {})",
                            list(outputs)
                        ),
                    );
                    return None;
                };
                let mut found: Vec<Option<usize>> = vec![None; types.names.len()];
                let mut ok = true;
                let what = format!("`{}` routes", name.value);
                for (part_type, conveyor) in &routes.0 {
                    let e = self.part_type(part_type, &part_type.value, types, &what);
                    let c = outgoing(self, conveyor);
                    match (e, c) {
                        (Some(e), Some(c)) => found[e] = Some(c),
                        _ => ok = false,
                    }
                }
                if !ok {
                    return None;
                }
                Routing::ByType(found)
            }
            raw::OutputRuleKind::RoundRobin => Routing::RoundRobin,
            raw::OutputRuleKind::FirstAvailable => Routing::FirstAvailable,
            raw::OutputRuleKind::ShortestQueue => Routing::ShortestQueue,
            raw::OutputRuleKind::Probability => {
                let Some(weights) = &rule.weights else {
                    self.error(
                        &rule.rule,
                        format!("`rule: probability` needs `weights` for {}", list(outputs)),
                    );
                    return None;
                };
                let mut found: Vec<Option<f64>> = vec![None; outputs.len()];
                let mut ok = true;
                for (conveyor, weight) in &weights.0 {
                    let Some(k) = outputs
                        .iter()
                        .position(|&c| conveyor_names[c] == conveyor.value)
                    else {
                        self.error(
                            conveyor,
                            format!(
                                "`{}` isn't an outgoing conveyor of `{}` (those are {})",
                                conveyor.value,
                                name.value,
                                list(outputs)
                            ),
                        );
                        ok = false;
                        continue;
                    };
                    if !(weight.value.is_finite() && weight.value > 0.0) {
                        self.error(weight, "weights must be greater than zero".to_string());
                        ok = false;
                    }
                    found[k] = Some(weight.value);
                }
                let missing: Vec<usize> = outputs
                    .iter()
                    .zip(&found)
                    .filter(|(_, w)| w.is_none())
                    .map(|(&c, _)| c)
                    .collect();
                if ok && !missing.is_empty() {
                    self.error(
                        &rule.rule,
                        format!("`weights` has no entry for {}", list(&missing)),
                    );
                    ok = false;
                }
                let weights: Vec<f64> = found.into_iter().map(|w| w.unwrap_or(0.0)).collect();
                let sum: f64 = weights.iter().sum();
                if ok && (sum - 1.0).abs() > 1e-6 {
                    self.error(&rule.rule, format!("`weights` add up to {sum}, not 1"));
                    ok = false;
                }
                if !ok {
                    return None;
                }
                Routing::Probability(weights)
            }
        };
        Some((outputs.to_vec(), routing))
    }

    /// The incoming conveyors of a server, in the order its rule uses them.
    /// A rule is required with two or more.
    fn input_rule(
        &mut self,
        name: &Spanned<String>,
        rule: &Option<Spanned<raw::RawInputRule>>,
        inputs: &[usize],
        conveyor_names: &[&str],
        names: &Names,
    ) -> Option<(Vec<usize>, InputRule)> {
        let list = |items: &[usize]| {
            items
                .iter()
                .map(|&c| format!("`{}`", conveyor_names[c]))
                .collect::<Vec<_>>()
                .join(", ")
        };
        if inputs.is_empty() {
            self.error(
                name,
                format!("server `{}` has no incoming conveyor", name.value),
            );
            return None;
        }
        let Some(rule) = rule else {
            if inputs.len() == 1 {
                return Some((inputs.to_vec(), InputRule::Single));
            }
            self.error(
                name,
                format!(
                    "server `{}` has {} incoming conveyors ({}); add `input: {{ rule: first_come | priority | round_robin | lowest | highest }}` to say which part it takes next",
                    name.value,
                    inputs.len(),
                    list(inputs)
                ),
            );
            return None;
        };
        let rule = &rule.value;
        let by_attribute = matches!(
            rule.rule.value,
            raw::InputRuleKind::Lowest | raw::InputRuleKind::Highest
        );
        match (&rule.by, by_attribute) {
            (None, true) => {
                self.error(
                    &rule.rule,
                    "`lowest` and `highest` need `by: <attribute>`, the attribute they compare"
                        .to_string(),
                );
                return None;
            }
            (Some(by), false) => {
                self.error(
                    by,
                    "`by` only applies to `rule: lowest` and `rule: highest`".to_string(),
                );
                return None;
            }
            _ => {}
        }
        match (rule.rule.value, &rule.order) {
            (raw::InputRuleKind::Priority, None) => {
                self.error(
                    &rule.rule,
                    format!(
                        "`rule: priority` needs `order`, listing {} from highest priority to lowest",
                        list(inputs)
                    ),
                );
                None
            }
            (raw::InputRuleKind::Priority, Some(order)) => {
                let mut ordered = Vec::new();
                let mut ok = true;
                for conveyor in &order.value {
                    match inputs
                        .iter()
                        .find(|&&c| conveyor_names[c] == conveyor.value)
                    {
                        None => {
                            self.error(
                                conveyor,
                                format!(
                                    "`{}` isn't an incoming conveyor of `{}` (those are {})",
                                    conveyor.value,
                                    name.value,
                                    list(inputs)
                                ),
                            );
                            ok = false;
                        }
                        Some(&c) if ordered.contains(&c) => {
                            self.error(conveyor, format!("`{}` is listed twice", conveyor.value));
                            ok = false;
                        }
                        Some(&c) => ordered.push(c),
                    }
                }
                let missing: Vec<usize> = inputs
                    .iter()
                    .copied()
                    .filter(|c| !ordered.contains(c))
                    .collect();
                if ok && !missing.is_empty() {
                    self.error(order, format!("`order` is missing {}", list(&missing)));
                    ok = false;
                }
                ok.then_some((ordered, InputRule::Priority))
            }
            (_, Some(order)) => {
                self.error(
                    order,
                    "`order` only applies to `rule: priority`".to_string(),
                );
                None
            }
            (raw::InputRuleKind::FirstCome, None) => Some((inputs.to_vec(), InputRule::FirstCome)),
            (kind @ (raw::InputRuleKind::Lowest | raw::InputRuleKind::Highest), None) => {
                let by = rule.by.as_ref().expect("checked above");
                let a = self.attribute(by, names)?;
                let rule = if kind == raw::InputRuleKind::Lowest {
                    InputRule::Lowest(a)
                } else {
                    InputRule::Highest(a)
                };
                Some((inputs.to_vec(), rule))
            }
            (raw::InputRuleKind::RoundRobin, None) => {
                Some((inputs.to_vec(), InputRule::RoundRobin))
            }
        }
    }

    fn experiment(&mut self, e: &raw::RawExperiment) -> Option<Experiment> {
        let replications = if e.replications.value == 0 {
            self.error(
                &e.replications,
                "`replications` must be at least 1".to_string(),
            );
            None
        } else {
            Some(e.replications.value)
        };
        let warmup = match &e.warmup {
            Some(w) => self.duration(w, true, "`warmup`"),
            None => Some(SimTime::ZERO),
        };
        let run_length = self.duration(&e.run_length, false, "`run_length`");
        Some(Experiment {
            replications: replications?,
            warmup: warmup?,
            run_length: run_length?,
            seed: e.seed,
        })
    }
}
