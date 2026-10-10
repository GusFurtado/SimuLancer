//! Validation: turns a [`RawModel`] into an engine [`Spec`], collecting every
//! problem instead of stopping at the first one.

use std::collections::HashMap;

use outliers_sim_engine::SimTime;
use outliers_sim_engine::spec::{
    Conveyor, Distribution, Experiment, Failure, FailureClock, InputRule, Node, OnFailure, PerType,
    Resource, Routing, Schedule, Server, Sink, Source, Spec, WhenBlocked,
};
use serde_saphyr::Spanned;

use crate::diagnostic::{Diagnostic, Position};
use crate::expr::{constant, parse_time_distribution};
use crate::raw::{self, RawModel, Scalar};
use crate::units::{DefaultUnits, Dimension, convert_number, parse_quantity};
use crate::{FORMAT, Model, Outputs};

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
    // conveyors and sinks share one, since conveyors refer to them by name.
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

    // Schedules have their own namespace: only `schedule:` fields name them.
    let mut schedules = Vec::new();
    for (name, s) in &raw.schedules.0 {
        b.check_name(name);
        if let Some(periods) = b.schedule(name, s) {
            schedules.push(Schedule {
                name: name.value.clone(),
                periods,
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
                        "`{}` is already used by a {}; names must be unique across sources, servers, conveyors, sinks and resources",
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
        let from = b.endpoint(&objects, name, &c.from, End::From);
        let to = b.endpoint(&objects, name, &c.to, End::To);
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
                from,
                to,
                length,
                speed,
                spacing,
                capacity,
            });
        }
    }

    let conveyor_names: Vec<&str> = raw
        .conveyors
        .0
        .iter()
        .map(|(n, _)| n.value.as_str())
        .collect();

    let mut sources = Vec::new();
    for (i, (name, s)) in raw.sources.0.iter().enumerate() {
        let creates = b.creates(name, &s.creates, &types);
        let interarrival = b.distribution(&s.interarrival);
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
                )
            })
            .flatten();
        if let (Some(creates), Some(interarrival), Some((outputs, routing)), Some(schedule)) =
            (creates, interarrival, output, schedule)
        {
            sources.push(Source {
                name: name.value.clone(),
                creates,
                interarrival,
                when_blocked: match s.when_blocked {
                    raw::WhenBlocked::Wait => WhenBlocked::Wait,
                    raw::WhenBlocked::Drop => WhenBlocked::Drop,
                },
                outputs,
                routing,
                schedule,
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
        resources.push(Resource {
            name: name.value.clone(),
            capacity: r.capacity.value,
            schedule,
        });
    }

    let mut servers = Vec::new();
    for (i, (name, s)) in raw.servers.0.iter().enumerate() {
        let capacity = match &s.capacity {
            Some(c) if c.value == 0 => {
                b.error(
                    c,
                    format!("server `{}` needs a capacity of at least 1", name.value),
                );
                None
            }
            Some(c) => Some(c.value),
            None => Some(1),
        };
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
        let input = endpoints_ok
            .then(|| b.input_rule(name, &s.input, &server_inputs[i], &conveyor_names))
            .flatten();
        let output = endpoints_ok
            .then(|| {
                b.output_rule(
                    name,
                    "server",
                    &s.output,
                    &server_outputs[i],
                    &conveyor_names,
                    &types,
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
        ) = (
            capacity,
            process_time,
            failures_ok,
            needs_ok,
            input,
            output,
            schedule,
        ) {
            servers.push(Server {
                name: name.value.clone(),
                capacity,
                process_time,
                needs,
                schedule,
                failures,
                inputs,
                input_rule,
                outputs,
                routing,
            });
        }
    }

    let mut sinks = Vec::new();
    for (i, (name, _)) in raw.sinks.0.iter().enumerate() {
        if endpoints_ok && sink_inputs[i].is_empty() {
            b.error(
                name,
                format!("sink `{}` has no incoming conveyor", name.value),
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

    if !b.diagnostics.is_empty() {
        // File order; errors without a position last.
        b.diagnostics.sort_by_key(|d| {
            d.position
                .map_or((true, 0, 0), |p| (false, p.line, p.column))
        });
        return Err(b.diagnostics);
    }

    let spec = Spec {
        entities: raw
            .entities
            .0
            .iter()
            .map(|(n, _)| n.value.clone())
            .collect(),
        sources,
        servers,
        conveyors,
        sinks,
        resources,
        schedules,
        experiment: experiment.expect("no diagnostics means the experiment is valid"),
    };
    check_reachability(&spec, &raw)?;
    check_part_types(&spec, &raw)?;

    Ok(Model {
        name: raw.name.value,
        description: raw.description,
        spec,
        outputs: Outputs {
            event_log: raw.outputs.event_log,
            summary: raw.outputs.summary,
        },
    })
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
    let mut at_server = vec![vec![false; n]; spec.servers.len()];
    let mut at_source = vec![vec![false; n]; spec.sources.len()];
    let mut stack: Vec<(Node, usize)> = Vec::new();
    for (i, s) in spec.sources.iter().enumerate() {
        for &(e, _) in &s.creates {
            at_source[i][e] = true;
            stack.push((Node::Source(i), e));
        }
    }
    while let Some((node, e)) = stack.pop() {
        let (outputs, routing) = match node {
            Node::Source(i) => (&spec.sources[i].outputs, &spec.sources[i].routing),
            Node::Server(i) => (&spec.servers[i].outputs, &spec.servers[i].routing),
            Node::Sink(_) => continue,
        };
        let next: Vec<usize> = match routing {
            Routing::ByType(routes) => routes[e].into_iter().collect(),
            _ => outputs.clone(),
        };
        for c in next {
            if let Node::Server(s) = spec.conveyors[c].to
                && !at_server[s][e]
            {
                at_server[s][e] = true;
                stack.push((Node::Server(s), e));
            }
        }
    }
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
    Sink,
    Resource,
}

impl Kind {
    fn describe(self) -> &'static str {
        match self {
            Kind::Source => "source",
            Kind::Server => "server",
            Kind::Conveyor => "conveyor",
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
                    "conveyor `{}` {verb} `{}`, which doesn't exist",
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
                    _ => "conveyors can't connect directly to other conveyors",
                };
                self.error(
                    target,
                    format!(
                        "conveyor `{}` can't have `{word}: {}`: {reason}",
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
    fn output_rule(
        &mut self,
        name: &Spanned<String>,
        kind: &str,
        rule: &Option<Spanned<raw::RawOutputRule>>,
        outputs: &[usize],
        conveyor_names: &[&str],
        types: &Types,
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
                    "{kind} `{}` has {} outgoing conveyors ({}); add `output: {{ rule: probability | round_robin | first_available | shortest_queue | by_type }}` to say how parts choose",
                    name.value,
                    outputs.len(),
                    list(outputs)
                ),
            );
            return None;
        };
        let rule = &rule.value;
        if rule.routes.is_some() && rule.rule.value != raw::OutputRuleKind::ByType {
            self.error(
                &rule.rule,
                "`routes` only apply to `rule: by_type`".to_string(),
            );
            return None;
        }
        let routing = match rule.rule.value {
            raw::OutputRuleKind::ByType => {
                if rule.weights.is_some() {
                    self.error(
                        &rule.rule,
                        "`weights` only apply to `rule: probability`".to_string(),
                    );
                    return None;
                }
                let Some(routes) = &rule.routes else {
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
                    let c = outputs
                        .iter()
                        .copied()
                        .find(|&c| conveyor_names[c] == conveyor.value);
                    if c.is_none() {
                        self.error(
                            conveyor,
                            format!(
                                "`{}` isn't an outgoing conveyor of `{}` (those are {})",
                                conveyor.value,
                                name.value,
                                list(outputs)
                            ),
                        );
                    }
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
            raw::OutputRuleKind::RoundRobin
            | raw::OutputRuleKind::FirstAvailable
            | raw::OutputRuleKind::ShortestQueue
                if rule.weights.is_some() =>
            {
                self.error(
                    &rule.rule,
                    "`weights` only apply to `rule: probability`".to_string(),
                );
                return None;
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
                    "server `{}` has {} incoming conveyors ({}); add `input: {{ rule: first_come | priority | round_robin }}` to say which part it takes next",
                    name.value,
                    inputs.len(),
                    list(inputs)
                ),
            );
            return None;
        };
        let rule = &rule.value;
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
