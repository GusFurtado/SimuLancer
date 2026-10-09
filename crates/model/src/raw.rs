//! The YAML file as written, before validation. Mirrors the file format
//! one-to-one; unknown keys are errors.

use std::fmt;
use std::marker::PhantomData;

use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde_saphyr::Spanned;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawModel {
    pub format: Spanned<String>,
    pub name: Spanned<String>,
    pub description: Option<String>,
    pub units: Option<RawUnits>,
    #[serde(default)]
    pub entities: OrderedMap<RawEntity>,
    #[serde(default)]
    pub sources: OrderedMap<RawSource>,
    #[serde(default)]
    pub servers: OrderedMap<RawServer>,
    #[serde(default)]
    pub conveyors: OrderedMap<RawConveyor>,
    #[serde(default)]
    pub sinks: OrderedMap<RawSink>,
    pub experiment: RawExperiment,
    #[serde(default)]
    pub outputs: RawOutputs,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawUnits {
    pub time: Option<Spanned<String>>,
    pub length: Option<Spanned<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawEntity {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawSource {
    pub creates: Spanned<String>,
    pub interarrival: Spanned<Scalar>,
    #[serde(default)]
    pub when_blocked: WhenBlocked,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhenBlocked {
    #[default]
    Wait,
    Drop,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawServer {
    pub capacity: Option<Spanned<u32>>,
    pub process_time: Spanned<Scalar>,
    #[serde(default)]
    pub failures: Vec<RawFailure>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawFailure {
    pub name: Spanned<String>,
    pub uptime: Spanned<Scalar>,
    pub downtime: Spanned<Scalar>,
    #[serde(default)]
    pub counts: FailureClock,
    #[serde(default)]
    pub on_failure: OnFailure,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClock {
    #[default]
    BusyTime,
    ClockTime,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnFailure {
    #[default]
    Resume,
    Restart,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawConveyor {
    pub from: Spanned<String>,
    pub to: Spanned<String>,
    #[serde(rename = "type", default)]
    pub kind: ConveyorKind,
    pub length: Spanned<Scalar>,
    pub speed: Spanned<Scalar>,
    pub spacing: Spanned<Scalar>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConveyorKind {
    #[default]
    Accumulating,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawSink {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawExperiment {
    pub replications: Spanned<u32>,
    pub warmup: Option<Spanned<Scalar>>,
    pub run_length: Spanned<Scalar>,
    pub seed: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawOutputs {
    #[serde(default)]
    pub event_log: EventLog,
    #[serde(default = "all_summary_items")]
    pub summary: Vec<SummaryItem>,
}

impl Default for RawOutputs {
    fn default() -> Self {
        RawOutputs {
            event_log: EventLog::default(),
            summary: all_summary_items(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventLog {
    #[default]
    None,
    Jsonl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryItem {
    Throughput,
    TimeInSystem,
    ServerStates,
    ConveyorOccupancy,
    SourceBlockedTime,
}

fn all_summary_items() -> Vec<SummaryItem> {
    vec![
        SummaryItem::Throughput,
        SummaryItem::TimeInSystem,
        SummaryItem::ServerStates,
        SummaryItem::ConveyorOccupancy,
        SummaryItem::SourceBlockedTime,
    ]
}

/// A value that can be written as a plain number (using the default unit)
/// or as a string with a unit or an expression.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Number(f64),
    Text(String),
}

/// A YAML map of names to objects that keeps file order, so object indices
/// (and therefore results) don't depend on hashing.
#[derive(Debug)]
pub struct OrderedMap<V>(pub Vec<(Spanned<String>, V)>);

impl<V> Default for OrderedMap<V> {
    fn default() -> Self {
        OrderedMap(Vec::new())
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for OrderedMap<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MapVisitor<V>(PhantomData<V>);

        impl<'de, V: Deserialize<'de>> Visitor<'de> for MapVisitor<V> {
            type Value = OrderedMap<V>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a map of names to objects")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(OrderedMap(entries))
            }
        }

        deserializer.deserialize_map(MapVisitor(PhantomData))
    }
}
