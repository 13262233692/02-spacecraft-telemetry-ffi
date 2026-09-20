//! Side-by-side diff of two decoded frames.

use frame_parser::model::DecodedFrame;
use serde_json::Value;

use crate::color::{bold, dim, paint, Style};

struct FieldEntry {
    key: String,
    region: &'static str,
    left: Option<Value>,
    right: Option<Value>,
}

fn primary_order(frame: &DecodedFrame) -> Vec<String> {
    frame
        .primary_fields
        .iter()
        .map(|(k, _)| k.clone())
        .collect()
}

pub fn render_diff(left: &DecodedFrame, right: &DecodedFrame) -> String {
    let mut out = String::new();
    out.push_str(&bold(&format!(
        "Frame Diff: #{} @ 0x{:06X}  vs  #{} @ 0x{:06X}",
        left.index, left.file_offset, right.index, right.file_offset
    )));
    out.push('\n');

    let mut all_keys: Vec<String> = Vec::new();
    for key in primary_order(left) {
        if !all_keys.contains(&key) {
            all_keys.push(key);
        }
    }
    for (key, _) in left.values.iter() {
        if !all_keys.contains(key) {
            all_keys.push(key.clone());
        }
    }
    for (key, _) in right.values.iter() {
        if !all_keys.contains(key) {
            all_keys.push(key.clone());
        }
    }

    let mut rows = Vec::new();
    for key in &all_keys {
        let l = left.get(key).cloned();
        let r = right.get(key).cloned();
        if l != r {
            rows.push(FieldEntry {
                key: key.clone(),
                region: region_of(key),
                left: l,
                right: r,
            });
        }
    }
    if rows.is_empty() {
        out.push_str(&paint(
            Style::Green,
            "  No field-level differences detected.",
        ));
        out.push('\n');
    } else {
        out.push_str(&format!(
            "  {} field(s) differ:\n",
            paint(Style::YellowBold, &rows.len().to_string())
        ));
        let key_width = rows.iter().map(|r| r.key.len()).max().unwrap_or(0).min(32);
        for row in &rows {
            out.push_str(&format!(
                "    {:<width$}  {} -> {}\n",
                paint(Style::White, &format!("{}:{}", row.region, row.key)),
                value_cell(row.left.as_ref(), Style::Red),
                value_cell(row.right.as_ref(), Style::Green),
                width = key_width
            ));
        }
    }

    // Packet presence differences.
    let l_apids: Vec<u16> = left.packets.iter().map(|p| p.apid).collect();
    let r_apids: Vec<u16> = right.packets.iter().map(|p| p.apid).collect();
    if l_apids != r_apids {
        out.push_str(&format!(
            "  {} APIDs: {:?} -> {:?}\n",
            paint(Style::YellowBold, "Packet set changed:"),
            l_apids,
            r_apids
        ));
    }

    // Frame counters summary.
    out.push_str(&dim(&format!(
        "    ({} fields / {} fields compared; {} vs {} packets)\n",
        left.values.len(),
        right.values.len(),
        left.packets.len(),
        right.packets.len()
    )));
    out
}

fn region_of(key: &str) -> &'static str {
    if key.starts_with("pkt_") {
        "packet"
    } else if key.starts_with("clcw_") {
        "ocf"
    } else if key.starts_with("sh_") {
        "secondary"
    } else {
        "primary"
    }
}

fn value_cell(value: Option<&Value>, style: Style) -> String {
    match value {
        None => paint(Style::Gray, "<absent>"),
        Some(Value::String(s)) => paint(style, s),
        Some(Value::Number(n)) => {
            if let Some(float) = n.as_f64() {
                paint(style, &format_number(float))
            } else {
                paint(style, &n.to_string())
            }
        }
        Some(other) => paint(style, &other.to_string()),
    }
}

fn format_number(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{n:.1}")
    } else {
        let rounded = (n * 1000.0).round() / 1000.0;
        format!("{rounded}")
    }
}
