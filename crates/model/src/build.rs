//! Validation: turns a [`RawModel`] into an engine [`Spec`], collecting every
//! problem instead of stopping at the first one.

use std::collections::HashMap;

use outliers_sim_engine::SimTime;
use outliers_sim_engine::spec::{
    Conveyor, Distribution, Experiment, Failure, FailureClock, InputRule, Node, OnFailure, Routing,
    Server, Sink, Source, Spec, WhenBlocked,
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
    ];
    for (names, kind) in &all_names {
        for (i, name) in names.iter().enumerate() {
            b.check_name(name);
            match objects.get(name.value.as_str()) {
                Some(existing) => b.error(
                    name,
                    format!(
                        "`{}` is already used by a {}; names must be unique across sources, servers, conveyors and sinks",
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
        let entity = match entity_index.get(s.creates.value.as_str()) {
            Some(&e) => Some(e),
            None => {
                b.error(
                    &s.creates,
                    format!(
                        "source `{}` creates `{}`, which isn't listed under `entities`",
                        name.value, s.creates.value
                    ),
                );
                None
            }
        };
        let interarrival = b.distribution(&s.interarrival);
        let output = endpoints_ok
            .then(|| {
                b.output_rule(
                    name,
                    "source",
                    &s.output,
                    &source_outputs[i],
                    &conveyor_names,
                )
            })
            .flatten();
        if let (Some(entity), Some(interarrival), Some((outputs, routing))) =
            (entity, interarrival, output)
        {
            sources.push(Source {
                name: name.value.clone(),
                entity,
                interarrival,
                when_blocked: match s.when_blocked {
                    raw::WhenBlocked::Wait => WhenBlocked::Wait,
                    raw::WhenBlocked::Drop => WhenBlocked::Drop,
                },
                outputs,
                routing,
            });
        }
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
        let process_time = b.distribution(&s.process_time);
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
                )
            })
            .flatten();
        if let (
            Some(capacity),
            Some(process_time),
            true,
            Some((inputs, input_rule)),
            Some((outputs, routing)),
        ) = (capacity, process_time, failures_ok, input, output)
        {
            servers.push(Server {
                name: name.value.clone(),
                capacity,
                process_time,
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
        experiment: experiment.expect("no diagnostics means the experiment is valid"),
    };
    check_reachability(&spec, &raw)?;

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

#[derive(Clone, Copy)]
enum Kind {
    Source,
    Server,
    Conveyor,
    Sink,
}

impl Kind {
    fn describe(self) -> &'static str {
        match self {
            Kind::Source => "source",
            Kind::Server => "server",
            Kind::Conveyor => "conveyor",
            Kind::Sink => "sink",
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
        let result = match &value.value {
            Scalar::Number(n) => {
                convert_number(*n, Dimension::Time, &self.defaults).and_then(constant)
            }
            Scalar::Text(t) => parse_time_distribution(t, &self.defaults),
        };
        result.map_err(|message| self.error(value, message)).ok()
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
                    "{kind} `{}` has {} outgoing conveyors ({}); add `output: {{ rule: probability | round_robin | first_available | shortest_queue }}` to say how parts choose",
                    name.value,
                    outputs.len(),
                    list(outputs)
                ),
            );
            return None;
        };
        let rule = &rule.value;
        let routing = match rule.rule.value {
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
