//! Parquet files: `replications.parquet` and the `events.parquet` event log.
//!
//! Written through the `parquet` crate's column API (no Arrow), Snappy
//! compressed, with dictionary encoding, so repeated names (event types,
//! objects) cost almost nothing. Rows are buffered and written in row groups.

use std::io::{self, Write};
use std::sync::Arc;

use parquet::basic::{Compression, LogicalType, Repetition, Type as Physical};
use parquet::data_type::{ByteArray, ByteArrayType, DoubleType, Int64Type};
use parquet::errors::ParquetError;
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;
use parquet::file::writer::SerializedFileWriter;
use parquet::schema::types::Type;
use serde_json::Value;
use simulancer_engine::spec::{Node, Spec};
use simulancer_engine::stats::Summary;
use simulancer_engine::{Observer, SimTime, TraceEvent};

use crate::measures::Measure;

/// Rows per row group: about 30 MB of buffered event log.
const ROW_GROUP: usize = 1 << 18;

#[derive(Clone, Copy)]
pub enum Kind {
    Double,
    Int,
    Text,
}

/// A cell of a row.
#[derive(Clone)]
pub enum Cell {
    Null,
    Double(f64),
    Int(i64),
    Text(ByteArray),
}

/// One column's buffered values: only the non-null ones, plus a definition
/// level per row for optional columns (1 present, 0 null).
struct Column {
    kind: Kind,
    optional: bool,
    doubles: Vec<f64>,
    ints: Vec<i64>,
    texts: Vec<ByteArray>,
    defs: Vec<i16>,
}

/// A Parquet file written row by row.
pub struct Table<W: Write + Send> {
    writer: SerializedFileWriter<W>,
    columns: Vec<Column>,
    rows: usize,
}

fn to_io(e: ParquetError) -> io::Error {
    io::Error::other(e)
}

impl<W: Write + Send> Table<W> {
    /// `columns`: name, kind, and whether it can be null. `metadata` goes in
    /// the file's key-value metadata.
    pub fn new(
        out: W,
        columns: &[(&str, Kind, bool)],
        metadata: Vec<(String, String)>,
    ) -> io::Result<Self> {
        let fields = columns
            .iter()
            .map(|&(name, kind, optional)| {
                let physical = match kind {
                    Kind::Double => Physical::DOUBLE,
                    Kind::Int => Physical::INT64,
                    Kind::Text => Physical::BYTE_ARRAY,
                };
                let mut field =
                    Type::primitive_type_builder(name, physical).with_repetition(if optional {
                        Repetition::OPTIONAL
                    } else {
                        Repetition::REQUIRED
                    });
                if let Kind::Text = kind {
                    field = field.with_logical_type(Some(LogicalType::String));
                }
                field.build().map(Arc::new)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(to_io)?;
        let schema = Type::group_type_builder("schema")
            .with_fields(fields)
            .build()
            .map_err(to_io)?;
        let properties = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .set_created_by(format!("simulancer {}", env!("CARGO_PKG_VERSION")))
            .set_key_value_metadata(Some(
                metadata
                    .into_iter()
                    .map(|(k, v)| KeyValue::new(k, v))
                    .collect(),
            ))
            .build();
        let writer = SerializedFileWriter::new(out, Arc::new(schema), Arc::new(properties))
            .map_err(to_io)?;
        Ok(Table {
            writer,
            columns: columns
                .iter()
                .map(|&(_, kind, optional)| Column {
                    kind,
                    optional,
                    doubles: Vec::new(),
                    ints: Vec::new(),
                    texts: Vec::new(),
                    defs: Vec::new(),
                })
                .collect(),
            rows: 0,
        })
    }

    /// Adds a row: one cell per column, in order.
    pub fn push(&mut self, row: &[Cell]) -> io::Result<()> {
        debug_assert_eq!(row.len(), self.columns.len());
        for (column, cell) in self.columns.iter_mut().zip(row) {
            let present = !matches!(cell, Cell::Null);
            match (column.kind, cell) {
                (_, Cell::Null) if column.optional => {}
                (Kind::Double, Cell::Double(v)) => column.doubles.push(*v),
                (Kind::Int, Cell::Int(v)) => column.ints.push(*v),
                (Kind::Text, Cell::Text(v)) => column.texts.push(v.clone()),
                _ => panic!("a cell doesn't fit its column"),
            }
            if column.optional {
                column.defs.push(i16::from(present));
            }
        }
        self.rows += 1;
        if self.rows == ROW_GROUP {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.rows == 0 {
            return Ok(());
        }
        let mut group = self.writer.next_row_group().map_err(to_io)?;
        for column in &mut self.columns {
            let mut writer = group
                .next_column()
                .map_err(to_io)?
                .expect("one writer per column");
            let defs = column.optional.then_some(&column.defs[..]);
            match column.kind {
                Kind::Double => {
                    writer
                        .typed::<DoubleType>()
                        .write_batch(&column.doubles, defs, None)
                }
                Kind::Int => writer
                    .typed::<Int64Type>()
                    .write_batch(&column.ints, defs, None),
                Kind::Text => {
                    writer
                        .typed::<ByteArrayType>()
                        .write_batch(&column.texts, defs, None)
                }
            }
            .map_err(to_io)?;
            writer.close().map_err(to_io)?;
            column.doubles.clear();
            column.ints.clear();
            column.texts.clear();
            column.defs.clear();
        }
        group.close().map_err(to_io)?;
        self.rows = 0;
        Ok(())
    }

    /// Writes what's buffered and the file's footer.
    pub fn finish(mut self) -> io::Result<()> {
        self.flush()?;
        let mut out = self.writer.into_inner().map_err(to_io)?;
        out.flush()
    }
}

fn text(s: &str) -> Cell {
    Cell::Text(ByteArray::from(s))
}

/// `replications.parquet`: the same rows as `replications.csv`, typed. A
/// value that isn't finite (a measure with nothing to average) is null.
pub fn write_replications<W: Write + Send>(
    out: W,
    model_name: &str,
    measures: &[(Measure, Summary)],
) -> io::Result<()> {
    let mut table = Table::new(
        out,
        &[
            ("replication", Kind::Int, false),
            ("object_type", Kind::Text, false),
            ("object", Kind::Text, false),
            ("measure", Kind::Text, false),
            ("unit", Kind::Text, false),
            ("value", Kind::Double, true),
        ],
        vec![(
            "simulancer".into(),
            serde_json::json!({ "format": "simulancer-replications/0.1", "model": model_name })
                .to_string(),
        )],
    )?;
    for (m, _) in measures {
        let (object_type, object, name, unit) = (
            text(m.object_type),
            text(&m.object),
            text(&m.name),
            text(m.unit.file_name()),
        );
        for (i, v) in m.values.iter().enumerate() {
            table.push(&[
                Cell::Int(i as i64 + 1),
                object_type.clone(),
                object.clone(),
                name.clone(),
                unit.clone(),
                if v.is_finite() {
                    Cell::Double(*v)
                } else {
                    Cell::Null
                },
            ])?;
        }
    }
    table.finish()
}

/// The columns of `events.parquet`: one row per event, with the keys of
/// `events.jsonl` as columns (null where an event type doesn't have one).
pub const EVENT_COLUMNS: &[(&str, Kind, bool)] = &[
    ("t", Kind::Double, false),
    ("type", Kind::Text, false),
    ("part", Kind::Int, true),
    ("source", Kind::Text, true),
    ("server", Kind::Text, true),
    ("sink", Kind::Text, true),
    ("conveyor", Kind::Text, true),
    ("entity", Kind::Text, true),
    ("slot", Kind::Int, true),
    ("resource", Kind::Text, true),
    ("units", Kind::Int, true),
    ("failure", Kind::Text, true),
    ("attribute", Kind::Text, true),
    ("value", Kind::Double, true),
    ("fleet", Kind::Text, true),
    ("vehicle", Kind::Int, true),
    ("from", Kind::Text, true),
    ("to", Kind::Text, true),
    ("until", Kind::Double, true),
];

/// Names as Parquet values, made once (cloning one is a reference count).
struct Names {
    sources: Vec<Cell>,
    servers: Vec<Cell>,
    sinks: Vec<Cell>,
    conveyors: Vec<Cell>,
    entities: Vec<Cell>,
    resources: Vec<Cell>,
    attributes: Vec<Cell>,
    fleets: Vec<Cell>,
    failures: Vec<Vec<Cell>>,
}

/// The event log as Parquet. The JSON header of `events.jsonl` goes in the
/// file's metadata, under `simulancer`.
pub struct ParquetEventLog<W: Write + Send> {
    table: Table<W>,
    names: Names,
    row: Vec<Cell>,
    error: Option<io::Error>,
}

impl<W: Write + Send> ParquetEventLog<W> {
    pub fn new(out: W, header: &Value, spec: &Spec) -> io::Result<Self> {
        let cells = |names: Vec<&String>| names.into_iter().map(|n| text(n)).collect::<Vec<_>>();
        let names = Names {
            sources: cells(spec.sources.iter().map(|s| &s.name).collect()),
            servers: cells(spec.servers.iter().map(|s| &s.name).collect()),
            sinks: cells(spec.sinks.iter().map(|s| &s.name).collect()),
            conveyors: cells(spec.conveyors.iter().map(|c| &c.name).collect()),
            entities: cells(spec.entities.iter().collect()),
            resources: cells(spec.resources.iter().map(|r| &r.name).collect()),
            attributes: cells(spec.attributes.iter().map(|a| &a.name).collect()),
            fleets: cells(spec.vehicles.iter().map(|f| &f.name).collect()),
            failures: spec
                .servers
                .iter()
                .map(|s| cells(s.failures.iter().map(|f| &f.name).collect()))
                .collect(),
        };
        let table = Table::new(
            out,
            EVENT_COLUMNS,
            vec![("simulancer".into(), header.to_string())],
        )?;
        Ok(ParquetEventLog {
            table,
            names,
            row: vec![Cell::Null; EVENT_COLUMNS.len()],
            error: None,
        })
    }

    /// Writes the rest and reports the first write error, if any.
    pub fn finish(mut self) -> io::Result<()> {
        if let Some(err) = self.error.take() {
            return Err(err);
        }
        self.table.finish()
    }

    fn write(&mut self, time: SimTime, event: TraceEvent) -> io::Result<()> {
        use TraceEvent as E;
        const T: usize = 0;
        const TYPE: usize = 1;
        const PART: usize = 2;
        const SOURCE: usize = 3;
        const SERVER: usize = 4;
        const SINK: usize = 5;
        const CONVEYOR: usize = 6;
        const ENTITY: usize = 7;
        const SLOT: usize = 8;
        const RESOURCE: usize = 9;
        const UNITS: usize = 10;
        const FAILURE: usize = 11;
        const ATTRIBUTE: usize = 12;
        const VALUE: usize = 13;
        const FLEET: usize = 14;
        const VEHICLE: usize = 15;
        const FROM: usize = 16;
        const TO: usize = 17;
        const UNTIL: usize = 18;

        let n = &self.names;
        let node = |node: Node| match node {
            Node::Source(i) => n.sources[i].clone(),
            Node::Server(i) => n.servers[i].clone(),
            Node::Sink(i) => n.sinks[i].clone(),
        };
        let row = &mut self.row;
        row.fill(Cell::Null);
        row[T] = Cell::Double(time.as_micros() as f64 / 1e6);
        let part = |p: u64| Cell::Int(p as i64);
        let kind = match event {
            E::VehicleTrip {
                fleet,
                vehicle,
                from,
                to,
                until,
                part: carried,
            } => {
                row[FLEET] = n.fleets[fleet].clone();
                row[VEHICLE] = Cell::Int(vehicle as i64);
                row[FROM] = node(from);
                row[TO] = node(to);
                row[UNTIL] = Cell::Double(until.as_micros() as f64 / 1e6);
                if let Some(p) = carried {
                    row[PART] = part(p);
                }
                "vehicle_trip"
            }
            E::Delivered { part: p, conveyor } => {
                row[PART] = part(p);
                row[CONVEYOR] = n.conveyors[conveyor].clone();
                "delivered"
            }
            E::Created {
                part: p,
                source,
                entity,
            } => {
                row[PART] = part(p);
                row[SOURCE] = n.sources[source].clone();
                row[ENTITY] = n.entities[entity].clone();
                "created"
            }
            E::GaveUp { part: p, conveyor } => {
                row[PART] = part(p);
                row[CONVEYOR] = n.conveyors[conveyor].clone();
                "gave_up"
            }
            E::Lost { part: p, source } => {
                row[PART] = part(p);
                row[SOURCE] = n.sources[source].clone();
                "lost"
            }
            E::EnteredConveyor { part: p, conveyor } => {
                row[PART] = part(p);
                row[CONVEYOR] = n.conveyors[conveyor].clone();
                "entered_conveyor"
            }
            E::ReachedEnd { part: p, conveyor } => {
                row[PART] = part(p);
                row[CONVEYOR] = n.conveyors[conveyor].clone();
                "reached_end"
            }
            E::StartedProcess {
                part: p,
                server,
                slot,
            } => {
                row[PART] = part(p);
                row[SERVER] = n.servers[server].clone();
                row[SLOT] = Cell::Int(slot as i64);
                "started_process"
            }
            E::ResourceTaken {
                resource,
                units,
                server,
                part: p,
            } => {
                row[RESOURCE] = n.resources[resource].clone();
                row[UNITS] = Cell::Int(i64::from(units));
                row[SERVER] = n.servers[server].clone();
                row[PART] = part(p);
                "resource_taken"
            }
            E::ResourceReturned {
                resource,
                units,
                server,
                part: p,
            } => {
                row[RESOURCE] = n.resources[resource].clone();
                row[UNITS] = Cell::Int(i64::from(units));
                row[SERVER] = n.servers[server].clone();
                row[PART] = part(p);
                "resource_returned"
            }
            E::FinishedProcess { part: p, server } => {
                row[PART] = part(p);
                row[SERVER] = n.servers[server].clone();
                "finished_process"
            }
            E::Failed { server, failure } => {
                row[SERVER] = n.servers[server].clone();
                row[FAILURE] = n.failures[server][failure].clone();
                "failed"
            }
            E::Repaired { server, failure } => {
                row[SERVER] = n.servers[server].clone();
                row[FAILURE] = n.failures[server][failure].clone();
                "repaired"
            }
            E::Collected { part: p, server } => {
                row[PART] = part(p);
                row[SERVER] = n.servers[server].clone();
                "collected"
            }
            E::Consumed { part: p, server } => {
                row[PART] = part(p);
                row[SERVER] = n.servers[server].clone();
                "consumed"
            }
            E::Assembled {
                part: p,
                server,
                entity,
            } => {
                row[PART] = part(p);
                row[SERVER] = n.servers[server].clone();
                row[ENTITY] = n.entities[entity].clone();
                "assembled"
            }
            E::Stocked {
                part: p,
                conveyor,
                entity,
            } => {
                row[PART] = part(p);
                row[CONVEYOR] = n.conveyors[conveyor].clone();
                row[ENTITY] = n.entities[entity].clone();
                "stocked"
            }
            E::Split {
                part: p,
                server,
                entity,
            } => {
                row[PART] = part(p);
                row[SERVER] = n.servers[server].clone();
                row[ENTITY] = n.entities[entity].clone();
                "split"
            }
            E::ShiftEnded { server } => {
                row[SERVER] = n.servers[server].clone();
                "shift_ended"
            }
            E::ShiftStarted { server } => {
                row[SERVER] = n.servers[server].clone();
                "shift_started"
            }
            E::Exited { part: p, sink } => {
                row[PART] = part(p);
                row[SINK] = n.sinks[sink].clone();
                "exited"
            }
            E::Set {
                part: p,
                attribute,
                value,
            } => {
                row[PART] = part(p);
                row[ATTRIBUTE] = n.attributes[attribute].clone();
                // events.jsonl writes a value that isn't finite as null.
                if value.is_finite() {
                    row[VALUE] = Cell::Double(value);
                }
                "set"
            }
        };
        row[TYPE] = Cell::Text(ByteArray::from(bytes::Bytes::from_static(kind.as_bytes())));
        self.table.push(row)
    }
}

impl<W: Write + Send> Observer for ParquetEventLog<W> {
    fn record(&mut self, time: SimTime, event: TraceEvent) {
        if self.error.is_none()
            && let Err(err) = self.write(time, event)
        {
            self.error = Some(err);
        }
    }
}
