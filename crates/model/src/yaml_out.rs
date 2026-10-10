//! Writes a model tree back as readable YAML, for files people will read and
//! diff: block-style maps and lists of maps, flow-style lists of plain values,
//! and quotes only where a value would otherwise be misread.
//!
//! Comments and custom formatting of the original file are not kept.

use serde_json::Value;

/// The tree as YAML text.
pub fn to_yaml(tree: &Value) -> String {
    lines_with_paths(tree)
        .into_iter()
        .map(|(line, _)| line)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// The YAML lines, each with the dotted path of the value it belongs to
/// (used to map errors in generated text back to what produced them).
pub(crate) fn lines_with_paths(tree: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Value::Object(map) = tree {
        emit_map(map, 0, "", &mut out);
    }
    out
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn emit_map(
    map: &serde_json::Map<String, Value>,
    indent: usize,
    path: &str,
    out: &mut Vec<(String, String)>,
) {
    let pad = " ".repeat(indent);
    for (key, value) in map {
        let child = join(path, key);
        let key_text = if is_plain_key(key) {
            key.clone()
        } else {
            Value::String(key.clone()).to_string()
        };
        match value {
            Value::Object(m) if !m.is_empty() => {
                out.push((format!("{pad}{key_text}:"), child.clone()));
                emit_map(m, indent + 2, &child, out);
            }
            Value::Array(items) if items.iter().any(is_block_item) => {
                out.push((format!("{pad}{key_text}:"), child.clone()));
                emit_list(items, indent + 2, &child, out);
            }
            _ => out.push((format!("{pad}{key_text}: {}", flow(value)), child)),
        }
    }
}

/// A list whose items include maps: one `- ` item per entry, maps in block style.
fn emit_list(items: &[Value], indent: usize, path: &str, out: &mut Vec<(String, String)>) {
    let pad = " ".repeat(indent);
    for (i, item) in items.iter().enumerate() {
        let child = join(path, &i.to_string());
        match item {
            Value::Object(m) if !m.is_empty() => {
                // Write the map two spaces deeper, then put the dash on its first line.
                let start = out.len();
                emit_map(m, indent + 2, &child, out);
                let first = &mut out[start].0;
                *first = format!("{pad}- {}", first.trim_start());
            }
            _ => out.push((format!("{pad}- {}", flow(item)), child)),
        }
    }
}

fn is_block_item(v: &Value) -> bool {
    matches!(v, Value::Object(m) if !m.is_empty())
}

/// A value on one line: plain or quoted scalars, `[a, b]`, `{ k: v }`.
fn flow(value: &Value) -> String {
    match value {
        Value::String(s) if is_plain(s) => s.clone(),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(flow).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Object(map) if map.is_empty() => "{}".into(),
        Value::Object(map) => {
            let parts: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let key = if is_plain_key(k) {
                        k.clone()
                    } else {
                        Value::String(k.clone()).to_string()
                    };
                    format!("{key}: {}", flow(v))
                })
                .collect();
            format!("{{ {} }}", parts.join(", "))
        }
        // Numbers, booleans, null and strings that need quotes: JSON is valid YAML.
        other => other.to_string(),
    }
}

fn is_plain_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && is_plain(key)
}

/// Strings that YAML reads back as the same string without quotes. Kept
/// deliberately narrow: anything doubtful gets quoted.
fn is_plain(s: &str) -> bool {
    let Some(first) = s.chars().next() else {
        return false;
    };
    let reserved = [
        "true", "false", "null", "yes", "no", "on", "off", "y", "n", "~",
    ];
    first.is_ascii_alphanumeric()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || " _-./()".contains(c))
        && !s.ends_with(' ')
        && !reserved.contains(&s.to_ascii_lowercase().as_str())
        && s.parse::<f64>().is_err()
        && !s.starts_with("0x")
        && !s.starts_with("0o")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn readable_output() {
        let tree = json!({
            "format": "outliers-sim/0.1",
            "entities": { "part": {} },
            "servers": {
                "mill": {
                    "capacity": 2,
                    "process_time": "triangular(1 min, 1.4 min, 2.1 min)",
                    "input": { "rule": "priority", "order": ["c_rework", "c_a"] },
                    "failures": [
                        { "name": "breakdown", "uptime": "exponential(1 h)", "counts": "busy_time" }
                    ]
                }
            },
            "conveyors": { "c": { "from": "a", "length": "12 m", "speed": "0.5 m/s" } },
            "tricky": ["yes", "12", "true", "a: b", "ok text"]
        });
        let text = to_yaml(&tree);
        assert_eq!(
            text,
            r#"format: outliers-sim/0.1
entities:
  part: {}
servers:
  mill:
    capacity: 2
    process_time: "triangular(1 min, 1.4 min, 2.1 min)"
    input:
      rule: priority
      order: [c_rework, c_a]
    failures:
      - name: breakdown
        uptime: exponential(1 h)
        counts: busy_time
conveyors:
  c:
    from: a
    length: 12 m
    speed: 0.5 m/s
tricky: ["yes", "12", "true", "a: b", ok text]
"#
        );
        // And it reads back as the same tree.
        let back: Value = serde_saphyr::from_str(&text).unwrap();
        assert_eq!(back, tree);
    }
}
