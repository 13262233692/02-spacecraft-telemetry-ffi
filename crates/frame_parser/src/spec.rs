//! Frame structure definition loaded dynamically from an external JSON file.
//!
//! The definition describes every decodeable region of a CCSDS TM transfer
//! frame: the fixed primary header, the optional transfer frame secondary
//! header, the operational control field and the embedded CCSDS space packets
//! extracted from the M_PDU multiplexed data field.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// JSON object keys arrive as strings; this helper accepts numeric-looking
/// keys (with optional `0x` prefix) for enum value maps.
mod enum_map {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::collections::BTreeMap;

    pub fn deserialize<'de, D>(deserializer: D) -> Result<BTreeMap<u64, String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = BTreeMap::<String, String>::deserialize(deserializer)?;
        raw.into_iter()
            .map(|(k, v)| {
                let key = k.trim().trim_start_matches("0x").trim_start_matches("0b");
                let parsed = if k.trim_start().starts_with("0x") {
                    u64::from_str_radix(key, 16)
                } else if k.trim_start().starts_with("0b") {
                    u64::from_str_radix(key, 2)
                } else {
                    key.parse::<u64>()
                };
                parsed
                    .map(|n| (n, v))
                    .map_err(|e| serde::de::Error::custom(format!("invalid enum key `{k}`: {e}")))
            })
            .collect()
    }

    pub fn serialize<S>(map: &BTreeMap<u64, String>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::Serialize;
        let converted: BTreeMap<String, &String> =
            map.iter().map(|(k, v)| (k.to_string(), v)).collect();
        converted.serialize(serializer)
    }
}

/// How the transfer frame secondary header length field is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LengthMode {
    /// Length field = number of secondary-header *data* octets minus one
    /// (CCSDS 132.0 convention); total header = 2 + (value + 1).
    #[default]
    DataMinusOne,
    /// Length field = total secondary header octets minus one.
    TotalMinusOne,
    /// Length field is already the total number of secondary header octets.
    TotalOctets,
}

/// Numeric rendering radix (only affects the display string).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Radix {
    #[default]
    Dec,
    Hex,
    Bin,
    Oct,
}

/// Reference point for a field's bit offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// Start of the region the field list belongs to.
    #[default]
    RegionStart,
    /// First octet of the whole transfer frame (after the ASM).
    FrameStart,
    /// Start of the multiplexed data field.
    DataFieldStart,
}

/// A single bit field declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSpec {
    /// Human readable name, also the key used by the rule engine.
    pub name: String,
    /// Reference point the offset is measured from.
    #[serde(default)]
    pub anchor: Anchor,
    /// Optional longer description shown in the parse report.
    #[serde(default)]
    pub description: Option<String>,
    /// Bit offset relative to the start of the enclosing region.
    pub offset: usize,
    /// Width in bits.
    pub bits: u32,
    /// Interpret the raw pattern as a two's complement signed integer.
    #[serde(default)]
    pub signed: bool,
    /// Linear conversion applied for display: value = raw * scale + offset.
    #[serde(default)]
    pub scale: Option<f64>,
    #[serde(default)]
    pub value_offset: Option<f64>,
    /// Engineering unit label, e.g. `V`, `A`, `degC`.
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub radix: Radix,
    /// Raw value -> symbolic label mapping.
    #[serde(default, with = "enum_map")]
    pub enum_values: BTreeMap<u64, String>,
}

/// Description of a fixed or optional frame region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderSpec {
    /// Region length in octets.
    pub length_octets: usize,
    /// Fields found in the region; offsets are region relative.
    #[serde(default)]
    pub fields: Vec<FieldSpec>,
}

/// The transfer frame secondary header is variable length. Its presence and
/// length are encoded in the primary-header transfer frame secondary header
/// flag and in the first two octets of the secondary header itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecondarySpec {
    pub fields: Vec<FieldSpec>,
    /// Bit offset (within the primary header) of the TF secondary header flag.
    pub presence_flag_bit: usize,
    /// Bit offset (region relative) of the 10-bit length field.
    #[serde(default = "default_len_bit")]
    pub length_bit: usize,
    #[serde(default = "default_len_bits")]
    pub length_bits: u32,
    #[serde(default)]
    pub length_mode: LengthMode,
}

fn default_len_bit() -> usize {
    4
}
fn default_len_bits() -> u32 {
    10
}

/// Operational Control Field (4 octets, typically a CLCW).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcfSpec {
    /// Primary-header bit offset of the OCF flag.
    pub presence_flag_bit: usize,
    #[serde(default)]
    pub fields: Vec<FieldSpec>,
}

/// Built-in semantic roles assigned to space packet primary-header fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PacketRole {
    Version,
    PacketType,
    SecHeaderFlag,
    Apid,
    SeqFlags,
    SeqCount,
    PacketLength,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacketFieldSpec {
    #[serde(flatten)]
    pub field: FieldSpec,
    /// When set, the field drives a built-in space packet parsing role.
    #[serde(default)]
    pub role: Option<PacketRole>,
}

/// Per-APID secondary header layout (offsets relative to the packet secondary
/// header start).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacketApidSpec {
    /// Human readable label used for flattened field keys.
    pub label: String,
    pub secondary_header_octets: usize,
    #[serde(default)]
    pub fields: Vec<FieldSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacketSpec {
    /// Fields of the 6-octet space packet primary header.
    pub primary_header: Vec<PacketFieldSpec>,
    /// APID reserved for idle packets.
    #[serde(default = "default_idle_apid")]
    pub idle_apid: u16,
    /// APID specific secondary headers.
    #[serde(default, rename = "secondary_by_apid")]
    pub secondary_by_apid: BTreeMap<u16, PacketApidSpec>,
}

fn default_idle_apid() -> u16 {
    0x7FF
}

fn default_version() -> u8 {
    0
}

/// Top-level frame structure definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Transfer frame version number stored in the primary header.
    #[serde(default = "default_version")]
    pub transfer_frame_version: u8,
    /// Fixed primary header (6 octets for TM).
    pub primary_header: HeaderSpec,
    /// Fixed nominal total frame length in octets.
    pub frame_length_octets: usize,
    /// Optional 4-octet Attached Synchronization Marker prefix hex pattern.
    #[serde(default)]
    pub asm: Option<String>,
    pub secondary_header: Option<SecondarySpec>,
    pub ocf: Option<OcfSpec>,
    /// Bit offset (primary header) of the 2-bit synchronization flag.
    pub sync_flag_bit: usize,
    /// Bit offset (primary header) of the 11-bit first header pointer.
    pub fhp_bit: usize,
    pub packet: Option<PacketSpec>,
    /// Fill byte used inside the idle/invalid data region.
    #[serde(default = "default_fill")]
    pub fill_byte: u8,
}

fn default_fill() -> u8 {
    0xE5
}

impl FrameSpec {
    pub fn from_json(json: &str) -> crate::error::Result<Self> {
        let spec: FrameSpec = serde_json::from_str(json)?;
        spec.validate()?;
        Ok(spec)
    }

    pub fn from_json_file(path: &std::path::Path) -> crate::error::Result<Self> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }

    fn validate(&self) -> crate::error::Result<()> {
        use crate::error::ParseError;
        if self.frame_length_octets < self.primary_header.length_octets {
            return Err(ParseError::Spec(
                "frame_length_octets must cover primary header".into(),
            ));
        }
        let asm_len = self
            .asm
            .as_ref()
            .map(|a| crate::bits::parse_hex(a).map_err(ParseError::Spec))
            .transpose()?
            .map(|v| v.len())
            .unwrap_or(0);
        if asm_len == 1 || asm_len > 16 {
            return Err(ParseError::Spec(
                "ASM must be absent or 2..=16 octets".into(),
            ));
        }
        for f in &self.primary_header.fields {
            check_field(f, self.primary_header.length_octets)?;
        }
        if let Some(sec) = &self.secondary_header {
            for f in &sec.fields {
                check_secondary_field(f, self.frame_length_octets)?;
            }
            if sec.length_bits != 10 {
                return Err(ParseError::Spec(
                    "secondary length field width must be 10".into(),
                ));
            }
        }
        if let Some(pkt) = &self.packet {
            let roles = [
                PacketRole::Version,
                PacketRole::PacketType,
                PacketRole::SecHeaderFlag,
                PacketRole::Apid,
                PacketRole::SeqFlags,
                PacketRole::SeqCount,
                PacketRole::PacketLength,
            ];
            for role in roles {
                let found = pkt.primary_header.iter().any(|f| f.role == Some(role));
                if !found {
                    return Err(ParseError::Spec(format!(
                        "space packet primary header is missing role `{role:?}`"
                    )));
                }
            }
            for f in &pkt.primary_header {
                check_field(&f.field, 6)?;
            }
            for apid_spec in pkt.secondary_by_apid.values() {
                for f in &apid_spec.fields {
                    check_field(f, apid_spec.secondary_header_octets)?;
                }
            }
        }
        Ok(())
    }
}

fn check_field(f: &FieldSpec, region_octets: usize) -> crate::error::Result<()> {
    use crate::error::ParseError;
    if f.bits == 0 || f.bits > 64 {
        return Err(ParseError::Spec(format!(
            "field `{}` width must be in 1..=64 bits",
            f.name
        )));
    }
    if f.offset + f.bits as usize > region_octets * 8 {
        return Err(ParseError::Spec(format!(
            "field `{}` (offset {} bits, width {}) exceeds region of {region_octets} octets",
            f.name, f.offset, f.bits
        )));
    }
    Ok(())
}

fn check_secondary_field(f: &FieldSpec, frame_octets: usize) -> crate::error::Result<()> {
    use crate::error::ParseError;
    if f.bits == 0 || f.bits > 64 {
        return Err(ParseError::Spec(format!(
            "field `{}` width must be in 1..=64 bits",
            f.name
        )));
    }
    if matches!(f.anchor, Anchor::FrameStart | Anchor::DataFieldStart) {
        // Absolute offset into the frame; validated against the frame length.
        if f.offset + f.bits as usize > frame_octets * 8 {
            return Err(ParseError::Spec(format!(
                "field `{}` exceeds frame length",
                f.name
            )));
        }
    }
    Ok(())
}

impl FieldSpec {
    /// Look up the symbolic label of a raw value, if an enum mapping exists.
    pub fn label_of(&self, raw: u64) -> Option<&str> {
        self.enum_values.get(&raw).map(String::as_str)
    }
}
