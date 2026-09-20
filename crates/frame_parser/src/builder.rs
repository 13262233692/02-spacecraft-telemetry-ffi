//! Test/sample TM transfer frame builder.
//!
//! This module is intentionally small: it produces spec-conformant frame
//! octets so the CLI can ship a ready-to-use offline example without a binary
//! blob. Production ingest goes through [`crate::decoder`].

use crate::bits::{parse_hex, write_bits};
use crate::error::{ParseError, Result};
use crate::spec::{Anchor, FieldSpec, FrameSpec};

/// A space packet to embed into the multiplexed data field.
#[derive(Debug, Clone)]
pub struct PacketInput {
    pub apid: u16,
    /// 0 = continuation, 1 = first, 2 = last, 3 = unsegmented.
    pub seq_flags: u8,
    pub seq_count: u16,
    pub sec_header_flag: bool,
    /// Secondary header + user data (everything after the 6-octet primary
    /// packet header).
    pub data: Vec<u8>,
}

impl PacketInput {
    /// Build an unsegmented packet with an optional secondary header.
    pub fn unsegmented(apid: u16, seq_count: u16, sec_header: Vec<u8>) -> Self {
        PacketInput {
            apid,
            seq_flags: 0b11,
            seq_count,
            sec_header_flag: !sec_header.is_empty(),
            data: sec_header,
        }
    }

    /// First segment of a multi-frame packet.
    pub fn first(apid: u16, seq_count: u16, data: Vec<u8>) -> Self {
        PacketInput {
            apid,
            seq_flags: 0b01,
            seq_count,
            sec_header_flag: false,
            data,
        }
    }

    /// Continuation / last segment of a multi-frame packet (no packet header
    /// of its own — these octets are raw data appended by the caller).
    pub fn continuation_data(data: &[u8]) -> Vec<u8> {
        data.to_vec()
    }

    /// Encode this packet into wire octets. The length field describes the
    /// whole logical packet, so a first segment's bytes can be sliced and the
    /// remainder sent in a continuation frame.
    pub fn to_wire(&self) -> Vec<u8> {
        self.encode()
    }

    fn encode(&self) -> Vec<u8> {
        // Packet data length = octets following the 6-octet primary header - 1.
        let total = self.data.len() as u16 - 1;
        // Word0: 3b version(0) | 1b type(0=TM) | 1b sec flag | 11b APID
        let word0: u16 = (u16::from(self.sec_header_flag as u8) << 11) | self.apid;
        let word1: u16 = ((self.seq_flags as u16) << 14) | (self.seq_count & 0x3FFF);
        let mut out = Vec::with_capacity(self.data.len() + 6);
        out.extend_from_slice(&word0.to_be_bytes());
        out.extend_from_slice(&word1.to_be_bytes());
        out.extend_from_slice(&total.to_be_bytes());
        out.extend_from_slice(&self.data);
        out
    }
}

/// Secondary header mission data expressed as `(field spec, raw value)`.
pub type FieldAssignment<'a> = (&'a FieldSpec, u64);

#[derive(Debug, Clone)]
pub struct FrameInput<'a> {
    pub spacecraft_id: u16,
    pub vcid: u8,
    pub master_frame_count: u8,
    pub vc_frame_count: u8,
    pub secondary_header_fields: Vec<FieldAssignment<'a>>,
    pub packets: Vec<PacketInput>,
    pub include_ocf: bool,
    pub ocf_word: u32,
}

impl<'a> FrameInput<'a> {
    pub fn nominal(spacecraft_id: u16, vcid: u8, master: u8, vc: u8) -> Self {
        FrameInput {
            spacecraft_id,
            vcid,
            master_frame_count: master,
            vc_frame_count: vc,
            secondary_header_fields: Vec::new(),
            packets: Vec::new(),
            include_ocf: false,
            ocf_word: 0,
        }
    }
}

/// Encode a single frame according to `spec`.
pub fn build_tm_frame<'a>(spec: &FrameSpec, input: &FrameInput<'a>) -> Result<Vec<u8>> {
    let ph_len = spec.primary_header.length_octets;
    let mut frame = vec![0u8; spec.frame_length_octets];

    // ---- primary header -----------------------------------------------------
    set_named_bits(
        &mut frame,
        spec,
        "transfer_frame_version",
        spec.transfer_frame_version as u64,
    )?;
    set_named_bits(
        &mut frame,
        spec,
        "spacecraft_id",
        input.spacecraft_id as u64,
    )?;
    set_named_bits(&mut frame, spec, "vcid", input.vcid as u64)?;
    set_named_bits(
        &mut frame,
        spec,
        "master_frame_count",
        input.master_frame_count as u64,
    )?;
    set_named_bits(
        &mut frame,
        spec,
        "vc_frame_count",
        input.vc_frame_count as u64,
    )?;

    // ---- secondary header ---------------------------------------------------
    let mut secondary_len = 0usize;
    if let Some(sec) = &spec.secondary_header {
        set_named_bits(&mut frame, spec, "secondary_header_flag", 1)?;
        // Determine the length needed for the declared fields, converting
        // every anchor into a frame-relative end position.
        let ph_end_bits = ph_len * 8;
        let max_end_bits = input
            .secondary_header_fields
            .iter()
            .map(|(f, _)| match f.anchor {
                Anchor::RegionStart => ph_end_bits + f.offset + f.bits as usize,
                Anchor::FrameStart => f.offset + f.bits as usize,
                // Data-field-relative fields lie after the header and cannot
                // influence its length.
                Anchor::DataFieldStart => ph_end_bits + 16,
            })
            .max()
            .unwrap_or(ph_end_bits + 16);
        secondary_len = (max_end_bits - ph_end_bits).div_ceil(8).max(2);
        let length_value = match sec.length_mode {
            // Data octets = total - 2; encoded value = data octets - 1.
            crate::spec::LengthMode::DataMinusOne => secondary_len.saturating_sub(3) as u64,
            crate::spec::LengthMode::TotalMinusOne => (secondary_len - 1) as u64,
            crate::spec::LengthMode::TotalOctets => secondary_len as u64,
        };
        write_bits(
            &mut frame[ph_len..ph_len + 2],
            sec.length_bit,
            sec.length_bits,
            length_value,
        );
        // Structural bits of the secondary header header (version/length/
        // spare) are managed above; skip any colliding user assignments.
        for (fs, value) in &input.secondary_header_fields {
            if matches!(fs.anchor, Anchor::FrameStart)
                && fs.offset >= ph_len * 8
                && fs.offset < ph_len * 8 + 16
            {
                continue;
            }
            let bit = match fs.anchor {
                Anchor::FrameStart => fs.offset,
                Anchor::RegionStart => ph_len * 8 + fs.offset,
                Anchor::DataFieldStart => (ph_len + secondary_len) * 8 + fs.offset,
            };
            write_bits(&mut frame, bit, fs.bits, *value);
        }
    }

    // ---- OCF flag (written before FHP so layout is final) -------------------
    if input.include_ocf {
        if let Some(ocf) = &spec.ocf {
            write_bits(&mut frame[..ph_len], ocf.presence_flag_bit, 1, 1);
        }
        let ocf_start = spec.frame_length_octets - 4;
        frame[ocf_start..ocf_start + 4].copy_from_slice(&input.ocf_word.to_be_bytes());
    }

    // ---- multiplexed data field --------------------------------------------
    let ocf_len = if input.include_ocf { 4 } else { 0 };
    let data_start = ph_len + secondary_len;
    let data_end = spec.frame_length_octets - ocf_len;
    let data_len = data_end - data_start;

    let packets: Vec<Vec<u8>> = input.packets.iter().map(|p| p.encode()).collect();
    let real_total: usize = packets.iter().map(Vec::len).sum();
    let fhp = if real_total > 0 {
        if real_total > data_len {
            return Err(ParseError::Packet(format!(
                "packets need {real_total} octets but data field holds {data_len}"
            )));
        }
        0u16
    } else {
        0x7FE
    };

    let mut cursor = data_start;
    if fhp == 0 {
        for packet in &packets {
            frame[cursor..cursor + packet.len()].copy_from_slice(packet);
            cursor += packet.len();
        }
        // Remaining space becomes an idle packet (APID 0x7FF), padded with the
        // configured fill byte (idle data payload content is not constrained).
        let remaining = data_end - cursor;
        if remaining >= 6 {
            let idle_len = remaining as u16 - 7;
            frame[cursor] = 0xFF;
            frame[cursor + 1] = 0xFF;
            frame[cursor + 2] = 0xC0; // unsegmented, seq count 0
            frame[cursor + 3] = 0x00;
            frame[cursor + 4..cursor + 6].copy_from_slice(&idle_len.to_be_bytes());
            cursor += 6;
            for b in &mut frame[cursor..data_end] {
                *b = spec.fill_byte;
            }
        } else {
            for b in &mut frame[cursor..data_end] {
                *b = spec.fill_byte;
            }
        }
    } else {
        for b in &mut frame[data_start..data_end] {
            *b = spec.fill_byte;
        }
    }

    // FHP + sync flag written last into the primary header.
    set_named_bits(&mut frame, spec, "sync_flag", 0)?;
    set_named_bits(&mut frame, spec, "first_header_pointer", fhp as u64)?;

    // ---- optional ASM prefix ------------------------------------------------
    if let Some(asm) = &spec.asm {
        let marker = parse_hex(asm).map_err(ParseError::Spec)?;
        let mut out = Vec::with_capacity(marker.len() + frame.len());
        out.extend_from_slice(&marker);
        out.extend_from_slice(&frame);
        return Ok(out);
    }
    Ok(frame)
}

fn set_named_bits(buf: &mut [u8], spec: &FrameSpec, name: &str, value: u64) -> Result<()> {
    let Some(fs) = spec.primary_header.fields.iter().find(|f| f.name == name) else {
        // Fields not declared in the JSON are simply skipped.
        return Ok(());
    };
    write_bits(buf, fs.offset, fs.bits, value);
    Ok(())
}
