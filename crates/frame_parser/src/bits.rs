//! Big-endian (network order) bit-level access helpers used by the CCSDS
//! decoder. CCSDS frames number bits starting at zero in the most significant
//! bit of the first octet.

use crate::error::{ParseError, Result};

/// Read `len` bits (1..=64) starting at bit offset `start` from `data`.
pub fn read_bits(data: &[u8], start: usize, len: u32) -> Result<u64> {
    if len == 0 || len > 64 {
        return Err(ParseError::Field {
            field: "<anonymous>".into(),
            reason: format!("bit width must be in 1..=64, got {len}"),
        });
    }
    let end = start + len as usize;
    let need_bytes = end.div_ceil(8);
    if data.len() < need_bytes {
        return Err(ParseError::Field {
            field: "<anonymous>".into(),
            reason: format!(
                "need {need_bytes} octets for bits {start}..{end}, have {}",
                data.len()
            ),
        });
    }
    let mut value: u64 = 0;
    for i in 0..len as usize {
        let bit = start + i;
        let byte = data[bit >> 3];
        let shift = 7 - (bit & 7);
        value = (value << 1) | u64::from((byte >> shift) & 1);
    }
    Ok(value)
}

/// Interpret `raw` (an `bits`-wide pattern) as a two's-complement signed int.
pub fn sign_extend(raw: u64, bits: u32) -> i64 {
    if bits < 64 && raw & (1u64 << (bits - 1)) != 0 {
        (raw | (!0u64 << bits)) as i64
    } else {
        raw as i64
    }
}

/// Write `len` bits of `value` at bit offset `start` into `buf` (big endian).
pub fn write_bits(buf: &mut [u8], start: usize, len: u32, value: u64) {
    for i in 0..len as usize {
        let bit = start + i;
        let src_shift = len as usize - 1 - i;
        let bit_value = (value >> src_shift) & 1;
        let dst_shift = 7 - (bit & 7);
        if bit_value == 1 {
            buf[bit >> 3] |= 1 << dst_shift;
        }
    }
}

pub fn u16_be(data: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([data[offset], data[offset + 1]])
}

pub fn u32_be(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Decode a hexadecimal string such as `1ACFFC1D`, optionally with `0x`.
pub fn parse_hex(input: &str) -> std::result::Result<Vec<u8>, String> {
    let cleaned = input
        .trim()
        .trim_start_matches("0x")
        .replace([' ', '_'], "");
    if cleaned.is_empty() || !cleaned.len().is_multiple_of(2) {
        return Err(format!("invalid hex string `{input}`"));
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

pub fn to_hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        s.push_str(&format!("{b:02X}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cross_byte_fields() {
        let data = [0b0001_1010, 0b1100_1111, 0b1101u8];
        assert_eq!(read_bits(&data, 0, 2).unwrap(), 0b00);
        assert_eq!(read_bits(&data, 2, 10).unwrap(), 0b0110101100);
        assert_eq!(read_bits(&data, 12, 10).unwrap(), 0b1111000011);
    }

    #[test]
    fn sign_extension() {
        assert_eq!(sign_extend(0b1111, 4), -1);
        assert_eq!(sign_extend(0b1000, 4), -8);
        assert_eq!(sign_extend(0b0111, 4), 7);
        assert_eq!(sign_extend(u64::MAX, 64), -1);
    }

    #[test]
    fn write_read_roundtrip() {
        let mut buf = [0u8; 4];
        write_bits(&mut buf, 3, 12, 0b101010101010);
        assert_eq!(read_bits(&buf, 3, 12).unwrap(), 0b101010101010);
    }
}
