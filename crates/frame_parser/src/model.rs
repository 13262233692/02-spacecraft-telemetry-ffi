//! Decoded representation of a frame: field values and extracted packets.

use serde_json::{json, Value};

use crate::spec::{FieldSpec, Radix};

/// A decoded field. `raw` always holds the bit pattern extracted from the
/// frame; `label` holds the enum symbolic name when a mapping exists.
#[derive(Debug, Clone)]
pub struct Field {
    pub spec_name: String,
    pub description: Option<String>,
    pub raw: u64,
    pub signed: bool,
    pub scale: Option<f64>,
    pub value_offset: Option<f64>,
    pub unit: Option<String>,
    pub label: Option<String>,
}

impl Field {
    pub fn from_spec(spec: &FieldSpec, raw: u64) -> Self {
        Field {
            spec_name: spec.name.clone(),
            description: spec.description.clone(),
            raw,
            signed: spec.signed,
            scale: spec.scale,
            value_offset: spec.value_offset,
            unit: spec.unit.clone(),
            label: spec.label_of(raw).map(str::to_string),
        }
    }

    /// Raw pattern interpreted as signed when the field is signed.
    pub fn signed_raw(&self, bits: u32) -> i64 {
        if self.signed {
            crate::bits::sign_extend(self.raw, bits)
        } else {
            self.raw as i64
        }
    }

    /// Numeric value after applying scale/offset.
    pub fn numeric(&self, bits: u32) -> f64 {
        let base = self.signed_raw(bits) as f64;
        base * self.scale.unwrap_or(1.0) + self.value_offset.unwrap_or(0.0)
    }

    /// Value best suited for rule comparisons: enum label takes precedence,
    /// otherwise the (possibly signed/scaled) numeric value.
    pub fn comparable(&self, bits: u32) -> Value {
        if let Some(label) = &self.label {
            Value::String(label.clone())
        } else if self.scale.is_some() || self.value_offset.is_some() {
            json!(self.numeric(bits))
        } else if self.signed {
            json!(self.signed_raw(bits))
        } else {
            json!(self.raw)
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DecodedPacket {
    pub apid: u16,
    pub label: Option<String>,
    pub seq_flags: u8,
    pub seq_count: u16,
    pub packet_length: u16,
    pub data_octets: usize,
    pub primary_fields: Vec<(String, Field)>,
    pub secondary_fields: Vec<(String, Field)>,
    pub payload_hex: String,
}

impl DecodedPacket {
    /// `true` for first segment / unsegmented packets.
    pub fn starts(&self) -> bool {
        self.seq_flags & 0b10 != 0
    }
    pub fn is_idle(&self, idle_apid: u16) -> bool {
        self.apid == idle_apid
    }
}

#[derive(Debug, Clone, Default)]
pub struct DecodedFrame {
    /// Zero-based frame index inside the input stream.
    pub index: usize,
    /// Offset of the ASM/frame start inside the input file.
    pub file_offset: usize,
    pub frame_octets: usize,
    pub primary_fields: Vec<(String, Field)>,
    pub secondary_fields: Vec<(String, Field)>,
    pub ocf_fields: Vec<(String, Field)>,
    pub packets: Vec<DecodedPacket>,
    /// Flat key -> value map consumed by the rule engine and JSON output.
    pub values: serde_json::Map<String, Value>,
    /// Width in bits of every flattened key, needed by Field comparisons.
    pub field_widths: std::collections::BTreeMap<String, u32>,
    /// Non-fatal decode problems (truncated fields, bad segment state...).
    pub warnings: Vec<String>,
}

impl DecodedFrame {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub fn display_string(key: &str, field: &Field, bits: u32, radix: Radix) -> String {
        if let Some(label) = &field.label {
            return label.clone();
        }
        let formatted = if field.scale.is_some() || field.value_offset.is_some() {
            format!("{:.3}", field.numeric(bits))
        } else if field.signed {
            field.signed_raw(bits).to_string()
        } else {
            match radix {
                Radix::Dec => field.raw.to_string(),
                Radix::Hex => format!("0x{:X}", field.raw),
                Radix::Bin => format!("0b{:b}", field.raw),
                Radix::Oct => format!("0o{:o}", field.raw),
            }
        };
        match &field.unit {
            Some(unit) => format!("{formatted} {unit}"),
            None => {
                let _ = key;
                formatted
            }
        }
    }
}
