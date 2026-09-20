//! Human-readable rendering of decoded transfer frames.

use frame_parser::model::DecodedFrame;
use frame_parser::spec::{FieldSpec, FrameSpec, Radix};

use crate::color::{bold, dim, paint, Style};

struct Row {
    key: String,
    value: String,
    raw: String,
    description: Option<String>,
}

pub fn render_frames(spec: &FrameSpec, frames: &[DecodedFrame]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{}\n",
        bold(&format!("TM Frame Parse Report — {}", spec.name))
    ));
    if let Some(desc) = &spec.description {
        out.push_str(&format!("{}\n", dim(desc)));
    }
    out.push('\n');
    for frame in frames {
        out.push_str(&render_frame(spec, frame));
        out.push('\n');
    }
    out
}

fn render_frame(spec: &FrameSpec, frame: &DecodedFrame) -> String {
    let mut out = String::new();
    let header = format!(
        "Frame #{} @ offset 0x{:06X} ({} octets)",
        frame.index, frame.file_offset, frame.frame_octets
    );
    out.push_str(&paint(Style::CyanBold, &header));
    out.push('\n');

    out.push_str(&render_section(
        "Primary Header",
        spec.primary_header.fields.iter().collect(),
        &frame.primary_fields,
    ));

    if let Some(sec_spec) = &spec.secondary_header {
        out.push_str(&render_section(
            "Secondary Header / Mission Data",
            sec_spec.fields.iter().collect(),
            &frame.secondary_fields,
        ));
    }

    if let Some(ocf_spec) = &spec.ocf {
        if !frame.ocf_fields.is_empty() {
            out.push_str(&render_section(
                "Operational Control Field (CLCW)",
                ocf_spec.fields.iter().collect(),
                &frame.ocf_fields,
            ));
        }
    }

    out.push_str(&render_packets(spec, frame));

    if !frame.warnings.is_empty() {
        out.push_str(&paint(
            Style::YellowBold,
            &format!("Warnings ({}):", frame.warnings.len()),
        ));
        out.push('\n');
        for warning in &frame.warnings {
            out.push_str(&format!("  {} {warning}\n", paint(Style::Yellow, "!")));
        }
    }
    out
}

fn render_section(
    title: &str,
    specs: Vec<&FieldSpec>,
    fields: &[(String, frame_parser::Field)],
) -> String {
    let mut rows = Vec::new();
    for (name, field) in fields {
        let spec = specs.iter().find(|f| &f.name == name);
        let bits = spec.map(|f| f.bits).unwrap_or(0);
        let radix = spec.map(|f| f.radix).unwrap_or_default();
        let raw_text = if field.label.is_some() {
            format!("{}", field.raw)
        } else {
            raw_radix(field.raw, radix)
        };
        rows.push(Row {
            key: name.clone(),
            value: frame_parser::model::DecodedFrame::display_string(name, field, bits, radix),
            raw: raw_text,
            description: field.description.clone(),
        });
    }
    format_table(title, &rows)
}

fn raw_radix(raw: u64, radix: Radix) -> String {
    match radix {
        Radix::Dec => format!("{raw}"),
        Radix::Hex => format!("0x{raw:X}"),
        Radix::Bin => format!("0b{raw:b}"),
        Radix::Oct => format!("0o{raw:o}"),
    }
}

fn format_table(title: &str, rows: &[Row]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str(&bold(&format!("  {title}")));
    out.push('\n');
    let key_width = rows.iter().map(|r| r.key.len()).max().unwrap_or(0).min(28);
    for row in rows {
        out.push_str(&format!(
            "    {:width$}  {:>14}  {}",
            paint(Style::White, &row.key),
            paint(Style::Green, &row.value),
            dim(&row.raw),
            width = key_width
        ));
        if let Some(desc) = &row.description {
            out.push_str(&format!("  {}", dim(&format!("// {desc}"))));
        }
        out.push('\n');
    }
    out
}

fn render_packets(spec: &FrameSpec, frame: &DecodedFrame) -> String {
    if frame.packets.is_empty() {
        return format!("  {}\n", dim("M_PDU: no space packets extracted"));
    }
    let mut out = String::new();
    let idle = spec.packet.as_ref().map(|p| p.idle_apid).unwrap_or(0x7FF);
    let real: Vec<_> = frame.packets.iter().filter(|p| p.apid != idle).collect();
    out.push_str(&bold(&format!(
        "  M_PDU Space Packets ({} real, {} idle)",
        real.len(),
        frame.packets.len() - real.len()
    )));
    out.push('\n');
    for packet in frame.packets.iter().filter(|p| p.apid != idle) {
        let title = format!(
            "    APID 0x{:03X}{} seq={} flags={} length={}",
            packet.apid,
            packet
                .label
                .as_ref()
                .map(|l| format!(" ({l})"))
                .unwrap_or_default(),
            packet.seq_count,
            seq_flag_label(packet.seq_flags),
            packet.data_octets
        );
        out.push_str(&paint(Style::MagentaBold, &title));
        out.push('\n');
        let key_width = packet
            .secondary_fields
            .iter()
            .map(|(n, _)| n.len())
            .max()
            .unwrap_or(0)
            .min(24);
        let pkt_spec = spec
            .packet
            .as_ref()
            .and_then(|p| p.secondary_by_apid.get(&packet.apid));
        for (name, field) in &packet.secondary_fields {
            let field_spec = pkt_spec.and_then(|ps| ps.fields.iter().find(|f| &f.name == name));
            let bits = field_spec.map(|f| f.bits).unwrap_or(0);
            let rendered = frame_parser::model::DecodedFrame::display_string(
                name,
                field,
                bits,
                field_spec.map(|f| f.radix).unwrap_or_default(),
            );
            out.push_str(&format!(
                "      {:width$}  {}\n",
                paint(Style::White, name),
                paint(Style::Green, &rendered),
                width = key_width
            ));
        }
        if !packet.payload_hex.is_empty() {
            let preview: String = packet.payload_hex.chars().take(32).collect();
            let suffix = if packet.payload_hex.len() > 32 {
                "..."
            } else {
                ""
            };
            out.push_str(&format!(
                "      {} {preview}{suffix} ({} payload octets)\n",
                dim("payload:"),
                packet.payload_hex.len() / 2
            ));
        }
    }
    out
}

fn seq_flag_label(flags: u8) -> &'static str {
    match flags {
        0 => "continuation",
        1 => "first",
        2 => "last",
        3 => "unsegmented",
        _ => "unknown",
    }
}
