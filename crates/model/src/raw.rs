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
    /// What flows through the model, in people's words: `{ one: customer, many: customers }`.
    pub parts: Option<RawWords>,
    pub units: Option<RawUnits>,
    #[serde(default)]
    pub entities: OrderedMap<RawEntity>,
    /// Values each part carries: name -> `{ default: .. }`.
    #[serde(default)]
    pub attributes: OrderedMap<RawAttribute>,
    #[serde(default)]
    pub sources: OrderedMap<RawSource>,
    #[serde(default)]
    pub servers: OrderedMap<RawServer>,
    #[serde(default)]
    pub conveyors: OrderedMap<RawConveyor>,
    #[serde(default)]
    pub queues: OrderedMap<RawQueue>,
    #[serde(default)]
    pub transports: OrderedMap<RawTransport>,
    #[serde(default)]
    pub vehicles: OrderedMap<RawFleet>,
    #[serde(default)]
    pub lines: OrderedMap<RawLine>,
    #[serde(default)]
    pub sinks: OrderedMap<RawSink>,
    #[serde(default)]
    pub resources: OrderedMap<RawResource>,
    #[serde(default)]
    pub schedules: OrderedMap<RawSchedule>,
    pub experiment: RawExperiment,
    #[serde(default)]
    pub outputs: RawOutputs,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawWords {
    pub one: Spanned<String>,
    pub many: Spanned<String>,
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
pub struct RawAttribute {
    /// A plain number (`0`) or a time (`0 s`): what the attribute holds.
    pub default: Spanned<Scalar>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawSource {
    /// One part type, or a mix: type -> share.
    pub creates: Spanned<RawCreates>,
    /// Time between arrivals; or `arrivals_per_hour`.
    pub interarrival: Option<Spanned<Scalar>>,
    /// Arrivals per hour by time of day: `"12:00-14:00": 70`.
    pub arrivals_per_hour: Option<Spanned<OrderedMap<f64>>>,
    /// Parts per arrival: `20`, or `{ min: 20, max: 30 }`.
    pub group: Option<Spanned<RawCount>>,
    /// Restocking instead of arrivals: the queue this source keeps stocked.
    pub restocks: Option<Spanned<String>>,
    pub reorder_point: Option<Spanned<u32>>,
    /// Parts per order; or `order_up_to`.
    pub order: Option<Spanned<u32>>,
    pub order_up_to: Option<Spanned<u32>>,
    pub lead_time: Option<Spanned<Scalar>>,
    pub review_every: Option<Spanned<Scalar>>,
    #[serde(default)]
    pub when_blocked: WhenBlocked,
    /// Required with two or more outgoing conveyors.
    pub output: Option<Spanned<RawOutputRule>>,
    pub schedule: Option<Spanned<String>>,
    /// Attribute -> formula, set on each new part.
    #[serde(default)]
    pub sets: OrderedMap<Spanned<Scalar>>,
}

/// A whole number of parts: `20`, or `{ min: 20, max: 30 }` (each value
/// equally likely). Read by hand: it's a number or a map.
#[derive(Debug, Clone, Copy)]
pub enum RawCount {
    Fixed(u64),
    Range { min: u64, max: u64 },
}

impl<'de> Deserialize<'de> for RawCount {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct CountVisitor;

        impl<'de> Visitor<'de> for CountVisitor {
            type Value = RawCount;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a whole number, or `{ min: .., max: .. }`")
            }

            fn visit_u64<E: serde::de::Error>(self, n: u64) -> Result<RawCount, E> {
                Ok(RawCount::Fixed(n))
            }

            fn visit_i64<E: serde::de::Error>(self, n: i64) -> Result<RawCount, E> {
                u64::try_from(n)
                    .map(RawCount::Fixed)
                    .map_err(|_| E::custom("a count can't be negative"))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RawCount, A::Error> {
                let (mut min, mut max) = (None, None);
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "min" => min = Some(map.next_value::<u64>()?),
                        "max" => max = Some(map.next_value::<u64>()?),
                        other => {
                            return Err(serde::de::Error::custom(format!(
                                "unknown field `{other}`, expected `min` and `max`"
                            )));
                        }
                    }
                }
                match (min, max) {
                    (Some(min), Some(max)) => Ok(RawCount::Range { min, max }),
                    _ => Err(serde::de::Error::custom(
                        "a range needs both `min` and `max`",
                    )),
                }
            }
        }

        deserializer.deserialize_any(CountVisitor)
    }
}

/// What a source creates: one part type, or a mix of types with shares.
/// (Positions are kept for the whole field: spans don't survive untagged
/// choices, so these deserialize by hand.)
#[derive(Debug)]
pub enum RawCreates {
    One(String),
    Mix(Vec<(String, f64)>),
}

/// A time that may differ by part type: one value, or type -> value.
#[derive(Debug)]
pub enum RawPerType {
    Same(Scalar),
    ByType(Vec<(String, Scalar)>),
}

impl<'de> Deserialize<'de> for RawCreates {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = RawCreates;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a part type, or a map of part types to shares")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<RawCreates, E> {
                Ok(RawCreates::One(v.to_string()))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RawCreates, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, f64>()? {
                    entries.push(entry);
                }
                Ok(RawCreates::Mix(entries))
            }
        }
        deserializer.deserialize_any(V)
    }
}

impl<'de> Deserialize<'de> for RawPerType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = RawPerType;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a time, or a map of part types to times")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<RawPerType, E> {
                Ok(RawPerType::Same(Scalar::Text(v.to_string())))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<RawPerType, E> {
                Ok(RawPerType::Same(Scalar::Number(v)))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<RawPerType, E> {
                Ok(RawPerType::Same(Scalar::Number(v as f64)))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<RawPerType, E> {
                Ok(RawPerType::Same(Scalar::Number(v as f64)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RawPerType, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, Scalar>()? {
                    entries.push(entry);
                }
                Ok(RawPerType::ByType(entries))
            }
        }
        deserializer.deserialize_any(V)
    }
}

/// How parts choose among outgoing conveyors.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawOutputRule {
    pub rule: Spanned<OutputRuleKind>,
    /// Conveyor name -> weight, for `probability`.
    pub weights: Option<OrderedMap<Spanned<f64>>>,
    /// Part type -> conveyor name, for `by_type`; a list of
    /// `{ when, to }`, for `by_condition`.
    pub routes: Option<Spanned<RawRoutes>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputRuleKind {
    Probability,
    RoundRobin,
    FirstAvailable,
    ShortestQueue,
    ByType,
    ByCondition,
}

/// `routes`: a map (part type -> conveyor) or a list of conditions. Read by
/// hand: it's a map or a list.
#[derive(Debug)]
pub enum RawRoutes {
    ByType(OrderedMap<Spanned<String>>),
    Conditions(Vec<RawConditionRoute>),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawConditionRoute {
    /// Absent on the last route, which takes every other part.
    pub when: Option<Spanned<Scalar>>,
    pub to: Spanned<String>,
}

impl<'de> Deserialize<'de> for RawRoutes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RoutesVisitor;

        impl<'de> Visitor<'de> for RoutesVisitor {
            type Value = RawRoutes;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("part type -> conveyor (`by_type`), or a list of `{ when, to }` (`by_condition`)")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RawRoutes, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(RawRoutes::ByType(OrderedMap(entries)))
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<RawRoutes, A::Error> {
                let mut routes = Vec::new();
                while let Some(route) = seq.next_element()? {
                    routes.push(route);
                }
                Ok(RawRoutes::Conditions(routes))
            }
        }

        deserializer.deserialize_any(RoutesVisitor)
    }
}

/// How a server chooses among parts waiting on incoming conveyors.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawInputRule {
    pub rule: Spanned<InputRuleKind>,
    /// Conveyor names, highest priority first, for `priority`.
    pub order: Option<Spanned<Vec<Spanned<String>>>>,
    /// The attribute `lowest` and `highest` compare.
    pub by: Option<Spanned<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputRuleKind {
    FirstCome,
    Priority,
    RoundRobin,
    Lowest,
    Highest,
}

/// Which part a queue lets go next: `first_in`, `last_in`,
/// `{ lowest: <attribute> }`, `{ highest: <attribute> }` or
/// `{ by_type: [<type>, ...] }`. Read by hand: it's a word or a map.
#[derive(Debug, Clone)]
pub enum RawQueueOrder {
    FirstIn,
    LastIn,
    Lowest(String),
    Highest(String),
    ByType(Vec<String>),
}

impl<'de> Deserialize<'de> for RawQueueOrder {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct OrderVisitor;

        const EXPECTED: &str = "`first_in`, `last_in`, `{ lowest: <attribute> }`, `{ highest: <attribute> }` or `{ by_type: [<part type>, ...] }`";

        impl<'de> Visitor<'de> for OrderVisitor {
            type Value = RawQueueOrder;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str(EXPECTED)
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<RawQueueOrder, E> {
                match v {
                    "first_in" => Ok(RawQueueOrder::FirstIn),
                    "last_in" => Ok(RawQueueOrder::LastIn),
                    other => Err(E::custom(format!(
                        "`{other}` isn't an order: use {EXPECTED}"
                    ))),
                }
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RawQueueOrder, A::Error> {
                let Some(key) = map.next_key::<String>()? else {
                    return Err(serde::de::Error::custom(format!(
                        "an empty order: use {EXPECTED}"
                    )));
                };
                let order = match key.as_str() {
                    "lowest" => RawQueueOrder::Lowest(map.next_value()?),
                    "highest" => RawQueueOrder::Highest(map.next_value()?),
                    "by_type" => RawQueueOrder::ByType(map.next_value()?),
                    other => {
                        return Err(serde::de::Error::custom(format!(
                            "unknown order `{other}`: use {EXPECTED}"
                        )));
                    }
                };
                if map.next_key::<String>()?.is_some() {
                    return Err(serde::de::Error::custom(format!(
                        "an order has one rule: {EXPECTED}"
                    )));
                }
                Ok(order)
            }
        }

        deserializer.deserialize_any(OrderVisitor)
    }
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
    pub process_time: Spanned<RawPerType>,
    /// Resource name -> units held while processing.
    #[serde(default)]
    pub needs: OrderedMap<Spanned<u32>>,
    /// Resource name -> units each part takes here and carries on.
    #[serde(default)]
    pub takes: OrderedMap<Spanned<u32>>,
    /// Resources whose carried units a part gives back when done here.
    pub gives_back: Option<Spanned<Vec<Spanned<String>>>>,
    pub schedule: Option<Spanned<String>>,
    /// Process this many parts together.
    pub batch: Option<Spanned<u32>>,
    /// Part type -> count: the kit an assembly server collects.
    pub assembles: Option<OrderedMap<Spanned<u32>>>,
    /// The part type an assembly makes.
    pub makes: Option<Spanned<String>>,
    /// The component whose arrival time the assembled part keeps.
    pub keeps_clock_of: Option<Spanned<String>>,
    /// Part type -> count: what each finished part becomes (one entry).
    pub splits_into: Option<Spanned<OrderedMap<RawCount>>>,
    /// Setup time for another part type: one time, or `from->to`: time.
    pub changeover: Option<Spanned<RawPerType>>,
    #[serde(default)]
    pub failures: Vec<RawFailure>,
    /// Required with two or more incoming conveyors.
    pub input: Option<Spanned<RawInputRule>>,
    /// Required with two or more outgoing conveyors.
    pub output: Option<Spanned<RawOutputRule>>,
    /// Attribute -> formula, set on each part when its processing finishes.
    #[serde(default)]
    pub sets: OrderedMap<Spanned<Scalar>>,
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

/// Parts wait here in order, with no travel time.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawQueue {
    pub from: Spanned<String>,
    pub to: Spanned<String>,
    /// Parts it can hold; unlimited when absent.
    pub capacity: Option<Spanned<u32>>,
    /// A service level: the share of parts that waited at most this long.
    pub target_wait: Option<Spanned<Scalar>>,
    /// Parts in it when the run starts: `150`, or `{ pallet: 150 }`.
    pub starts_with: Option<Spanned<RawStartsWith>>,
    /// Which part it lets go next; first in, first out by default.
    pub order: Option<Spanned<RawQueueOrder>>,
    /// How long a part waits before giving up: one time, or one per part type.
    pub patience: Option<Spanned<RawPerType>>,
}

/// Parts carried from one object to another by a vehicle.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawTransport {
    pub from: Spanned<String>,
    pub to: Spanned<String>,
    /// The fleet that carries them.
    pub by: Spanned<String>,
    pub distance: Spanned<Scalar>,
    /// Parts waiting, carried and delivered at once; unlimited when absent.
    pub capacity: Option<Spanned<u32>>,
}

/// A paced line: stations that move their parts on together every takt.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawLine {
    /// Servers, in the order parts go through them.
    pub stations: Spanned<Vec<String>>,
    pub takt: Spanned<Scalar>,
}

/// Vehicles: forklifts, carts, AGVs.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawFleet {
    pub count: Spanned<u32>,
    pub speed: Spanned<Scalar>,
    pub load_time: Option<Spanned<Scalar>>,
    pub unload_time: Option<Spanned<Scalar>>,
    /// Where they are when the run starts.
    pub home: Spanned<String>,
}

/// `150`, or `{ pallet: 150 }` when the part type has to be named.
#[derive(Debug)]
pub enum RawStartsWith {
    Count(u64),
    Typed(OrderedMap<u64>),
}

impl<'de> Deserialize<'de> for RawStartsWith {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StartsWithVisitor;

        impl<'de> Visitor<'de> for StartsWithVisitor {
            type Value = RawStartsWith;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a whole number, or `{ part_type: count }`")
            }

            fn visit_u64<E: serde::de::Error>(self, n: u64) -> Result<RawStartsWith, E> {
                Ok(RawStartsWith::Count(n))
            }

            fn visit_i64<E: serde::de::Error>(self, n: i64) -> Result<RawStartsWith, E> {
                u64::try_from(n)
                    .map(RawStartsWith::Count)
                    .map_err(|_| E::custom("a count can't be negative"))
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<RawStartsWith, A::Error> {
                Deserialize::deserialize(serde::de::value::MapAccessDeserializer::new(map))
                    .map(RawStartsWith::Typed)
            }
        }

        deserializer.deserialize_any(StartsWithVisitor)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawSink {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawResource {
    pub capacity: Spanned<u32>,
    pub schedule: Option<Spanned<String>>,
    /// Servers that get units first, highest priority first.
    pub order: Option<Spanned<Vec<Spanned<String>>>>,
}

/// Working times that repeat every week.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawSchedule {
    /// "06:00-14:00"; a shift ending at or before its start ends the next day.
    pub shifts: Spanned<Vec<String>>,
    /// mon, tue, ...; every day if absent.
    pub days: Option<Spanned<Vec<String>>>,
    /// Servers and resources on it finish the work in hand after a shift.
    #[serde(default)]
    pub finish_work: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawExperiment {
    pub replications: Spanned<u32>,
    pub warmup: Option<Spanned<Scalar>>,
    pub run_length: Spanned<Scalar>,
    pub seed: u64,
    /// A percentage: `5%`.
    pub precision: Option<Spanned<Scalar>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawOutputs {
    #[serde(default)]
    pub event_log: EventLog,
    #[serde(default = "all_summary_items")]
    pub summary: Vec<SummaryItem>,
    /// What throughput is counted per: `hour` (the default), `day`, `week`,
    /// or a schedule's name for per working hour of that schedule.
    pub throughput_per: Option<Spanned<String>>,
}

impl Default for RawOutputs {
    fn default() -> Self {
        RawOutputs {
            event_log: EventLog::default(),
            summary: all_summary_items(),
            throughput_per: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_type_values_keep_the_field_position() {
        #[derive(Deserialize)]
        struct T {
            a: Spanned<RawPerType>,
            b: Spanned<RawPerType>,
            c: Spanned<RawCreates>,
            d: Spanned<RawCreates>,
        }
        let t: T = serde_saphyr::from_str(
            "a: 5 min\nb: { x: \"1 min\", y: 2 }\nc: part\nd: { x: 0.25, y: 0.75 }\n",
        )
        .unwrap();
        assert!(matches!(t.a.value, RawPerType::Same(Scalar::Text(ref x)) if x == "5 min"));
        let RawPerType::ByType(b) = &t.b.value else {
            panic!()
        };
        assert_eq!(b[1].0, "y");
        assert!(matches!(b[1].1, Scalar::Number(n) if n == 2.0));
        assert_eq!(t.b.referenced.line(), 2);
        assert!(matches!(t.c.value, RawCreates::One(ref x) if x == "part"));
        assert_eq!(t.c.referenced.line(), 3);
        let RawCreates::Mix(d) = &t.d.value else {
            panic!()
        };
        assert_eq!(d[0], ("x".to_string(), 0.25));
    }
}
