//! CCSDS TM 帧解析引擎。
//!
//! 负责两件事：
//! 1. 解析固定 6 字节的 CCSDS 主帧头（Transfer Frame Primary Header）。
//! 2. 根据外部 JSON 描述的帧结构定义，从副帧头 / 数据域中按位提取字段。

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use anyhow::{ensure, Context, Result};
use serde::Deserialize;

/// CCSDS TM 主帧头（6 字节，CCSDS 132.0-B）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimaryHeader {
    /// 传输帧版本号（2 bit，TM 固定为 0b00）
    pub version: u8,
    /// 航天器标识（10 bit）
    pub spacecraft_id: u16,
    /// 虚拟信道 ID（3 bit）
    pub virtual_channel_id: u8,
    /// 是否携带操作控制域 OCF
    pub ocf_flag: bool,
    /// 主信道帧计数（8 bit）
    pub master_channel_frame_count: u8,
    /// 虚拟信道帧计数（8 bit）
    pub virtual_channel_frame_count: u8,
    /// 是否存在副帧头
    pub secondary_header_flag: bool,
    /// 同步标志（0 = 数据域含包，1 = 含 VCA 数据）
    pub sync_flag: bool,
    /// 包序号标志
    pub packet_order_flag: bool,
    /// 段长度标识（2 bit）
    pub segment_length_id: u8,
    /// 帧数据域首包头指针（11 bit）
    pub first_header_pointer: u16,
}

impl PrimaryHeader {
    pub const LEN: usize = 6;

    pub fn parse(data: &[u8]) -> Result<Self> {
        ensure!(
            data.len() >= Self::LEN,
            "帧长度 {} 不足主帧头长度 {}",
            data.len(),
            Self::LEN
        );
        let w0 = u16::from_be_bytes([data[0], data[1]]);
        let w1 = u16::from_be_bytes([data[4], data[5]]);
        Ok(PrimaryHeader {
            version: (w0 >> 14) as u8,
            spacecraft_id: ((w0 >> 4) & 0x3FF) as u16,
            virtual_channel_id: ((w0 >> 1) & 0x7) as u8,
            ocf_flag: (w0 & 0x1) != 0,
            master_channel_frame_count: data[2],
            virtual_channel_frame_count: data[3],
            secondary_header_flag: (w1 >> 15) & 0x1 != 0,
            sync_flag: (w1 >> 14) & 0x1 != 0,
            packet_order_flag: (w1 >> 13) & 0x1 != 0,
            segment_length_id: ((w1 >> 11) & 0x3) as u8,
            first_header_pointer: w1 & 0x7FF,
        })
    }
}

/// 字段数据类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    /// 无符号整数（默认）
    #[default]
    Uint,
    /// 二进制补码有符号整数
    Int,
    /// IEEE-754 单精度（bit_width = 32）
    F32,
    /// IEEE-754 双精度（bit_width = 64）
    F64,
    /// 枚举：提取无符号整数后查映射表
    Enum,
    /// 原始字节串（bit_width 须为 8 的倍数）
    Bytes,
}

/// JSON 帧结构定义中的单个字段。
#[derive(Debug, Clone, Deserialize)]
pub struct FieldDef {
    /// 字段名（规则引擎按此名引用）
    pub name: String,
    /// 相对所在区段起始的字节偏移
    #[serde(default)]
    pub byte_offset: usize,
    /// 字节内位偏移（MSB 优先，0 表示该字节最高位）
    #[serde(default)]
    pub bit_offset: usize,
    /// 位宽（1..=64）
    pub bit_width: usize,
    #[serde(rename = "type", default)]
    pub kind: FieldKind,
    /// 工程单位（仅展示用）
    pub unit: Option<String>,
    /// 物理值 = raw * scale + bias
    pub scale: Option<f64>,
    #[serde(default)]
    pub bias: f64,
    /// 枚举映射：{"0": "OFF", "1": "ON"}
    #[serde(rename = "enum", default)]
    pub enum_map: HashMap<String, String>,
}

/// 帧中的一个区段（副帧头 / 数据域）。
#[derive(Debug, Clone, Deserialize)]
pub struct Section {
    /// 区段在帧中的绝对起始字节
    pub start_byte: usize,
    pub fields: Vec<FieldDef>,
}

/// 顶层帧结构定义（由 JSON 动态加载）。
#[derive(Debug, Clone, Deserialize)]
pub struct FrameDefinition {
    pub name: String,
    /// 帧总长度（字节），用于校验输入
    pub total_bytes: usize,
    #[serde(default = "default_primary_header_len")]
    pub primary_header_bytes: usize,
    pub secondary_header: Option<Section>,
    pub data_field: Section,
}

fn default_primary_header_len() -> usize {
    PrimaryHeader::LEN
}

impl FrameDefinition {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取帧定义失败: {}", path.display()))?;
        let def: FrameDefinition = serde_json::from_str(&text)
            .with_context(|| format!("解析帧定义 JSON 失败: {}", path.display()))?;
        def.validate()?;
        Ok(def)
    }

    fn validate(&self) -> Result<()> {
        ensure!(self.total_bytes > 0, "total_bytes 必须大于 0");
        ensure!(
            self.primary_header_bytes == PrimaryHeader::LEN,
            "仅支持标准 6 字节主帧头"
        );
        for section in [&self.data_field].into_iter().chain(self.secondary_header.iter()) {
            for f in &section.fields {
                ensure!(
                    f.bit_width >= 1 && f.bit_width <= 64,
                    "字段 {} 位宽 {} 非法",
                    f.name,
                    f.bit_width
                );
                ensure!(f.bit_offset < 8, "字段 {} 位偏移越界", f.name);
                let end =
                    section.start_byte + f.byte_offset + (f.bit_offset + f.bit_width + 7) / 8;
                ensure!(
                    end <= self.total_bytes,
                    "字段 {} 超出帧总长 {}",
                    f.name,
                    self.total_bytes
                );
            }
        }
        Ok(())
    }
}

/// 解析后的字段值。
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    UInt(u64),
    Int(i64),
    Float(f64),
    Enum { raw: u64, label: String },
    Bytes(Vec<u8>),
}

impl FieldValue {
    /// 数值化视图，供规则引擎比较。
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            FieldValue::UInt(v) => Some(*v as f64),
            FieldValue::Int(v) => Some(*v as f64),
            FieldValue::Float(v) => Some(*v),
            FieldValue::Enum { raw, .. } => Some(*raw as f64),
            FieldValue::Bytes(_) => None,
        }
    }

    /// 字符串化视图（枚举返回标签），供等值比较。
    pub fn as_label(&self) -> Option<&str> {
        match self {
            FieldValue::Enum { label, .. } => Some(label),
            _ => None,
        }
    }
}

impl fmt::Display for FieldValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldValue::UInt(v) => write!(f, "{v}"),
            FieldValue::Int(v) => write!(f, "{v}"),
            FieldValue::Float(v) => write!(f, "{v:.3}"),
            FieldValue::Enum { raw, label } => write!(f, "{label} ({raw})"),
            FieldValue::Bytes(b) => {
                write!(f, "0x")?;
                for byte in b {
                    write!(f, "{byte:02X}")?;
                }
                Ok(())
            }
        }
    }
}

/// 一帧的完整解析结果。
#[derive(Debug)]
pub struct ParsedFrame {
    pub header: PrimaryHeader,
    /// 保持定义顺序的 (字段名, 值, 单位) 列表
    pub fields: Vec<(String, FieldValue, Option<String>)>,
}

impl ParsedFrame {
    pub fn get(&self, name: &str) -> Option<&FieldValue> {
        self.fields
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, v, _)| v)
    }
}

/// 按 MSB 优先从字节流中提取 start_bit 起的 width 个位。
pub fn extract_bits(data: &[u8], start_bit: usize, width: usize) -> Result<u64> {
    ensure!(width <= 64, "位宽 {width} 超过 64");
    let mut value: u64 = 0;
    for i in 0..width {
        let bit_index = start_bit + i;
        let byte = *data
            .get(bit_index / 8)
            .with_context(|| format!("位偏移 {bit_index} 超出帧长度 {}", data.len()))?;
        let bit = (byte >> (7 - (bit_index % 8))) & 1;
        value = (value << 1) | u64::from(bit);
    }
    Ok(value)
}

fn sign_extend(raw: u64, width: usize) -> i64 {
    if width == 64 {
        raw as i64
    } else {
        let shift = 64 - width;
        ((raw << shift) as i64) >> shift
    }
}

fn decode_field(data: &[u8], base: usize, def: &FieldDef) -> Result<FieldValue> {
    let start_bit = (base + def.byte_offset) * 8 + def.bit_offset;
    let apply_scale = |raw: f64| def.scale.map_or(raw, |s| raw * s + def.bias);
    match def.kind {
        FieldKind::Uint => {
            let raw = extract_bits(data, start_bit, def.bit_width)?;
            Ok(if def.scale.is_some() {
                FieldValue::Float(apply_scale(raw as f64))
            } else {
                FieldValue::UInt(raw)
            })
        }
        FieldKind::Int => {
            let raw = sign_extend(extract_bits(data, start_bit, def.bit_width)?, def.bit_width);
            Ok(if def.scale.is_some() {
                FieldValue::Float(apply_scale(raw as f64))
            } else {
                FieldValue::Int(raw)
            })
        }
        FieldKind::F32 => {
            ensure!(def.bit_width == 32, "字段 {}: f32 位宽须为 32", def.name);
            let raw = extract_bits(data, start_bit, 32)? as u32;
            Ok(FieldValue::Float(f32::from_bits(raw) as f64))
        }
        FieldKind::F64 => {
            ensure!(def.bit_width == 64, "字段 {}: f64 位宽须为 64", def.name);
            let raw = extract_bits(data, start_bit, 64)?;
            Ok(FieldValue::Float(f64::from_bits(raw)))
        }
        FieldKind::Enum => {
            let raw = extract_bits(data, start_bit, def.bit_width)?;
            let label = def
                .enum_map
                .get(&raw.to_string())
                .cloned()
                .unwrap_or_else(|| "UNKNOWN".to_string());
            Ok(FieldValue::Enum { raw, label })
        }
        FieldKind::Bytes => {
            ensure!(
                def.bit_width % 8 == 0,
                "字段 {}: bytes 类型位宽须为 8 的倍数",
                def.name
            );
            let mut out = Vec::with_capacity(def.bit_width / 8);
            for i in 0..def.bit_width / 8 {
                out.push(extract_bits(data, start_bit + i * 8, 8)? as u8);
            }
            Ok(FieldValue::Bytes(out))
        }
    }
}

/// 按定义解析一整帧。
pub fn parse_frame(def: &FrameDefinition, frame: &[u8]) -> Result<ParsedFrame> {
    ensure!(
        frame.len() == def.total_bytes,
        "帧长度 {} 与定义 {} 不符",
        frame.len(),
        def.total_bytes
    );
    let header = PrimaryHeader::parse(frame)?;
    let mut fields = Vec::new();

    let mut sections = Vec::new();
    if header.secondary_header_flag {
        if let Some(sec) = &def.secondary_header {
            sections.push(sec);
        }
    }
    sections.push(&def.data_field);

    for section in sections {
        for fdef in &section.fields {
            let value = decode_field(frame, section.start_byte, fdef)
                .with_context(|| format!("解析字段 {} 失败", fdef.name))?;
            fields.push((fdef.name.clone(), value, fdef.unit.clone()));
        }
    }

    Ok(ParsedFrame { header, fields })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_header_roundtrip() {
        // version=0, scid=0x2A, vcid=5, ocf=1; mc=0x11, vc=0x22; shflag=1, fhp=0x012
        let raw = [0x02, 0xAB, 0x11, 0x22, 0x80, 0x12];
        let h = PrimaryHeader::parse(&raw).unwrap();
        assert_eq!(h.version, 0);
        assert_eq!(h.spacecraft_id, 0x2A);
        assert_eq!(h.virtual_channel_id, 5);
        assert!(h.ocf_flag);
        assert_eq!(h.master_channel_frame_count, 0x11);
        assert_eq!(h.virtual_channel_frame_count, 0x22);
        assert!(h.secondary_header_flag);
        assert_eq!(h.first_header_pointer, 0x12);
    }

    #[test]
    fn extract_bits_msb_first() {
        let data = [0b1011_0011, 0b0100_0001];
        assert_eq!(extract_bits(&data, 0, 4).unwrap(), 0b1011);
        assert_eq!(extract_bits(&data, 4, 8).unwrap(), 0b0011_0100);
        assert_eq!(extract_bits(&data, 9, 7).unwrap(), 0b100_0001);
    }

    #[test]
    fn sign_extend_negative() {
        assert_eq!(sign_extend(0b1110, 4), -2);
        assert_eq!(sign_extend(0xFF, 8), -1);
        assert_eq!(sign_extend(0x7F, 8), 127);
    }

    #[test]
    fn decode_scaled_and_enum() {
        let json = r#"{
            "name": "T", "total_bytes": 8,
            "data_field": { "start_byte": 6, "fields": [
                {"name": "v", "byte_offset": 0, "bit_width": 8, "type": "uint", "scale": 0.5},
                {"name": "m", "byte_offset": 1, "bit_offset": 0, "bit_width": 2,
                 "type": "enum", "enum": {"0": "OFF", "1": "ON"}}
            ]}
        }"#;
        let def: FrameDefinition = serde_json::from_str(json).unwrap();
        def.validate().unwrap();
        let mut frame = vec![0u8; 8];
        frame[6] = 48; // v = 24.0
        frame[7] = 0b0100_0000; // m = 1 -> ON
        let parsed = parse_frame(&def, &frame).unwrap();
        assert_eq!(parsed.get("v"), Some(&FieldValue::Float(24.0)));
        assert_eq!(
            parsed.get("m"),
            Some(&FieldValue::Enum {
                raw: 1,
                label: "ON".into()
            })
        );
    }
}
