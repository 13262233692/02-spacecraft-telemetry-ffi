//! CCSDS TM transfer frame decoder driven entirely by a [`FrameSpec`].
//!
//! The decoder scans one or more concatenated frames (optionally prefixed by
//! an Attached Synchronization Marker), decodes every declared bit field and
//! reassembles CCSDS space packets out of the multiplexed data field.

use std::collections::{BTreeMap, HashMap};

use crate::bits::{read_bits, u16_be};
use crate::error::{ParseError, Result};
use crate::model::{DecodedFrame, DecodedPacket, Field};
use crate::spec::{Anchor, FrameSpec, PacketRole, PacketSpec};

/// Decode every frame found in `input`.
pub fn decode_stream(spec: &FrameSpec, input: &[u8]) -> Result<Vec<DecodedFrame>> {
    let mut decoder = Decoder::new(spec);
    let frames_len = spec.frame_length_octets;
    let asm = spec
        .asm
        .as_ref()
        .map(|a| crate::bits::parse_hex(a).expect("validated asm"))
        .unwrap_or_default();
    let asm_len = asm.len();
    let stride = frames_len + asm_len;

    let mut pos = 0usize;
    let mut index = 0usize;
    let mut frames = Vec::new();
    while pos < input.len() {
        if !asm.is_empty() {
            if pos + asm_len > input.len() {
                return Err(truncated(index, pos, asm_len, input.len() - pos));
            }
            if input[pos..pos + asm_len] != asm[..] {
                return Err(ParseError::Spec(format!(
                    "frame #{index} at offset {pos}: ASM mismatch"
                )));
            }
        }
        let frame_start = pos + asm_len;
        if frame_start + frames_len > input.len() {
            return Err(truncated(
                index,
                frame_start,
                frames_len,
                input.len() - frame_start,
            ));
        }
        let frame = &input[frame_start..frame_start + frames_len];
        frames.push(decoder.decode_frame(index, frame_start, frame)?);
        pos += stride;
        index += 1;
    }
    // Flush incomplete packets as warnings on the last frame.
    decoder.flush_pending(&mut frames);
    Ok(frames)
}

fn truncated(index: usize, offset: usize, need: usize, have: usize) -> ParseError {
    ParseError::Truncated {
        index,
        offset,
        need,
        have,
    }
}

struct Decoder<'a> {
    spec: &'a FrameSpec,
    /// Reassembly state keyed by virtual channel id (`gvcid` without APID).
    pending: HashMap<u16, PendingPacket>,
}

#[derive(Clone)]
struct PendingPacket {
    apid: u16,
    seq_count: u16,
    #[allow(dead_code)]
    seq_flags: u8,
    data: Vec<u8>,
}

impl<'a> Decoder<'a> {
    fn new(spec: &'a FrameSpec) -> Self {
        Decoder {
            spec,
            pending: HashMap::new(),
        }
    }

    fn decode_frame(
        &mut self,
        index: usize,
        file_offset: usize,
        frame: &[u8],
    ) -> Result<DecodedFrame> {
        let mut out = DecodedFrame {
            index,
            file_offset,
            frame_octets: frame.len(),
            ..Default::default()
        };

        let ph_len = self.spec.primary_header.length_octets;
        let primary = &frame[..ph_len];

        // ---- primary header fields -----------------------------------------
        for fs in &self.spec.primary_header.fields {
            let raw = read_field(primary, fs)?;
            let field = Field::from_spec(fs, raw);
            flatten(&mut out, &fs.name, raw, fs.bits, &field);
            out.primary_fields.push((fs.name.clone(), field));
        }

        // ---- secondary header ----------------------------------------------
        let mut secondary_len = 0usize;
        let mut secondary_present = false;
        if let Some(sec) = &self.spec.secondary_header {
            let flag = read_bits(primary, sec.presence_flag_bit, 1)?;
            if flag == 1 {
                secondary_present = true;
                // Length field lives in the first 2 octets of the secondary
                // header itself.
                let sh_start = ph_len;
                if sh_start + 2 > frame.len() {
                    return Err(ParseError::Field {
                        field: "<secondary header>".into(),
                        reason: "secondary header flag set but frame too short".into(),
                    });
                }
                let length_value = read_bits(&frame[sh_start..], sec.length_bit, sec.length_bits)?;
                secondary_len = match sec.length_mode {
                    crate::spec::LengthMode::DataMinusOne => 2 + (length_value as usize + 1),
                    crate::spec::LengthMode::TotalMinusOne => length_value as usize + 1,
                    crate::spec::LengthMode::TotalOctets => length_value as usize,
                };
                if sh_start + secondary_len > frame.len() {
                    return Err(ParseError::Field {
                        field: "<secondary header>".into(),
                        reason: format!(
                            "declared {secondary_len} octets but only {} remain",
                            frame.len() - sh_start
                        ),
                    });
                }
                for fs in &sec.fields {
                    match read_anchored(frame, fs, ph_len, secondary_len) {
                        Ok(raw) => {
                            let field = Field::from_spec(fs, raw);
                            flatten(&mut out, &fs.name, raw, fs.bits, &field);
                            out.secondary_fields.push((fs.name.clone(), field));
                        }
                        Err(reason) => out
                            .warnings
                            .push(format!("secondary field `{}`: {reason}", fs.name)),
                    }
                }
            }
        }
        let _ = secondary_present;

        // ---- OCF ------------------------------------------------------------
        let mut ocf_len = 0usize;
        if let Some(ocf) = &self.spec.ocf {
            let flag = read_bits(primary, ocf.presence_flag_bit, 1)?;
            if flag == 1 {
                ocf_len = 4;
                let ocf_start = frame.len() - ocf_len;
                for fs in &ocf.fields {
                    match read_bits(&frame[ocf_start..], fs.offset, fs.bits) {
                        Ok(raw) => {
                            let field = Field::from_spec(fs, raw);
                            flatten(&mut out, &fs.name, raw, fs.bits, &field);
                            out.ocf_fields.push((fs.name.clone(), field));
                        }
                        Err(reason) => out
                            .warnings
                            .push(format!("OCF field `{}`: {reason}", fs.name)),
                    }
                }
            }
        }

        // ---- multiplexed data field ----------------------------------------
        if let Some(pkt_spec) = &self.spec.packet {
            let sync = read_bits(primary, self.spec.sync_flag_bit, 2)?;
            let fhp = read_bits(primary, self.spec.fhp_bit, 11)?;
            let data_start = ph_len + secondary_len;
            let data_end = frame.len() - ocf_len;
            self.extract_packets(
                &mut out,
                pkt_spec,
                &frame[data_start..data_end],
                sync as u8,
                fhp as u16,
            )?;
        }

        Ok(out)
    }

    fn flush_pending(&self, frames: &mut [DecodedFrame]) {
        if self.pending.is_empty() || frames.is_empty() {
            return;
        }
        let last = frames.last_mut().unwrap();
        for (vc, pending) in &self.pending {
            last.warnings.push(format!(
                "virtual channel {vc}: unterminated packet APID {} (seq count {}) with {} octets",
                pending.apid,
                pending.seq_count,
                pending.data.len()
            ));
        }
    }
}

/// Read a field from a region using its declared anchor.
fn read_anchored(
    frame: &[u8],
    fs: &crate::spec::FieldSpec,
    ph_len: usize,
    secondary_len: usize,
) -> Result<u64> {
    match fs.anchor {
        Anchor::RegionStart | Anchor::DataFieldStart => {
            // For secondary fields the region starts at the secondary header.
            let start = match fs.anchor {
                Anchor::DataFieldStart => ph_len + secondary_len,
                _ => ph_len,
            };
            let bit = start * 8 + fs.offset;
            read_bits(frame, bit, fs.bits).map_err(|e| map_field_err(&fs.name, e))
        }
        Anchor::FrameStart => {
            read_bits(frame, fs.offset, fs.bits).map_err(|e| map_field_err(&fs.name, e))
        }
    }
}

fn read_field(region: &[u8], fs: &crate::spec::FieldSpec) -> Result<u64> {
    read_bits(region, fs.offset, fs.bits).map_err(|e| map_field_err(&fs.name, e))
}

fn map_field_err(name: &str, err: ParseError) -> ParseError {
    match err {
        ParseError::Field { reason, .. } => ParseError::Field {
            field: name.to_string(),
            reason,
        },
        other => other,
    }
}

fn flatten(out: &mut DecodedFrame, name: &str, raw: u64, bits: u32, field: &Field) {
    out.values.insert(name.to_string(), field.comparable(bits));
    out.field_widths.insert(name.to_string(), bits);
    let _ = raw;
}

impl<'a> Decoder<'a> {
    /// Process one multiplexed data field. `sync` is the 2-bit synchronisation
    /// flag; `fhp` the 11-bit first header pointer (0x7FE = idle data,
    /// 0x7FF = no packet start).
    fn extract_packets(
        &mut self,
        out: &mut DecodedFrame,
        pkt_spec: &PacketSpec,
        data: &[u8],
        sync: u8,
        fhp: u16,
    ) -> Result<()> {
        const FHP_IDLE: u16 = 0x7FE;
        const FHP_NO_START: u16 = 0x7FF;

        // Virtual channel identity for reassembly: the 10-bit GVCID from the
        // transfer frame primary header.
        let vc = virtual_channel(self.spec, out).unwrap_or(0u16);

        if sync == 0b10 || sync == 0b01 {
            // VCDU / bitstream or reserved: packet extraction not applicable.
            return Ok(());
        }

        if fhp == FHP_IDLE {
            // Entire data field is idle fill. A previously pending packet is
            // necessarily broken.
            if let Some(pending) = self.pending.remove(&vc) {
                out.warnings.push(format!(
                    "virtual channel {vc}: idle data while packet APID {} was pending",
                    pending.apid
                ));
            }
            return Ok(());
        }

        if fhp != FHP_NO_START {
            let split = fhp as usize;
            if split > data.len() {
                return Err(ParseError::Packet(format!(
                    "first header pointer {split} beyond data field of {} octets",
                    data.len()
                )));
            }
            let (continuation, rest) = data.split_at(split);

            // 1) Append continuation octets to an in-progress packet.
            if !continuation.is_empty() {
                match self.pending.get_mut(&vc) {
                    Some(pending) => pending.data.extend_from_slice(continuation),
                    None => out.warnings.push(format!(
                        "virtual channel {vc}: {split} continuation octet(s) without a pending packet"
                    )),
                }
            }

            // 2) Walk complete packet headers starting at the FHP.
            self.walk_packets(out, pkt_spec, rest, vc)?;
        } else {
            // No packet start: the whole data field continues one packet.
            match self.pending.get_mut(&vc) {
                Some(pending) => pending.data.extend_from_slice(data),
                None if !data.is_empty() => out.warnings.push(format!(
                    "virtual channel {vc}: continuation frame of {} octets without a pending packet",
                    data.len()
                )),
                _ => {}
            }
        }

        // Promote any packets that reached their declared length.
        self.drain_completed(out, pkt_spec, vc);
        Ok(())
    }

    fn walk_packets(
        &mut self,
        out: &mut DecodedFrame,
        pkt_spec: &PacketSpec,
        mut region: &[u8],
        vc: u16,
    ) -> Result<()> {
        while region.len() >= 6 {
            let roles = parse_packet_roles(pkt_spec, region)?;
            // Space packet data length field = number of octets after the
            // 6-octet primary header minus one.
            let total = (roles.packet_length as usize) + 1 + 6;
            if total > region.len() {
                // Packet continues in a later frame.
                if roles.seq_flags == 0b11 {
                    out.warnings.push(format!(
                        "virtual channel {vc}: unsegmented packet (APID {}) exceeds data field",
                        roles.apid
                    ));
                }
                let mut data = Vec::with_capacity(region.len());
                data.extend_from_slice(region);
                self.pending.insert(
                    vc,
                    PendingPacket {
                        apid: roles.apid,
                        seq_count: roles.seq_count,
                        seq_flags: roles.seq_flags,
                        data,
                    },
                );
                return Ok(());
            }

            let packet_bytes = &region[..total];
            if roles.apid == pkt_spec.idle_apid {
                // Idle packet; anything following inside this region is
                // non-conformant, so stop walking.
                break;
            }

            // Completion without spanning frames (seq flags permit continuation
            // only across frame boundaries).
            let decoded = build_packet(pkt_spec, packet_bytes, out)?;
            out.packets.push(decoded);
            region = &region[total..];

            // Stop once we reach idle/fill bytes.
            if region.first().copied() == Some(self.spec.fill_byte)
                || (region.len() >= 2 && u16_be(region, 0) == 0xFFFF)
            {
                break;
            }
        }
        Ok(())
    }

    fn drain_completed(&mut self, out: &mut DecodedFrame, pkt_spec: &PacketSpec, vc: u16) {
        // Take ownership so build_packet can mutate the frame's flat maps.
        let Some(pending) = self.pending.remove(&vc) else {
            return;
        };
        if pending.data.len() < 6 {
            self.pending.insert(vc, pending);
            return;
        }
        let roles = match parse_packet_roles(pkt_spec, &pending.data) {
            Ok(r) => r,
            Err(e) => {
                out.warnings.push(format!(
                    "virtual channel {vc}: pending packet unreadable: {e}"
                ));
                return;
            }
        };
        // Data length field semantics: octets after primary header minus one.
        let total = roles.packet_length as usize + 1 + 6;
        if pending.data.len() < total {
            // Not complete yet; restore it for subsequent frames.
            self.pending.insert(vc, pending);
            return;
        }
        let packet_bytes = pending.data[..total].to_vec();
        let extra = pending.data.len() - total;
        let unexpected_trailing = pending.data[total..]
            .iter()
            .any(|b| *b != self.spec.fill_byte);
        if roles.apid != pkt_spec.idle_apid {
            match build_packet(pkt_spec, &packet_bytes, out) {
                Ok(decoded) => out.packets.push(decoded),
                Err(e) => out
                    .warnings
                    .push(format!("packet APID {}: {e}", roles.apid)),
            }
        }
        if extra > 0 && unexpected_trailing {
            out.warnings.push(format!(
                "virtual channel {vc}: {extra} unexpected trailing octet(s) after completed packet"
            ));
        }
    }
}

struct PacketRolesRaw {
    apid: u16,
    seq_flags: u8,
    seq_count: u16,
    packet_length: u16,
    sec_header_flag: u8,
}

fn parse_packet_roles(pkt_spec: &PacketSpec, bytes: &[u8]) -> Result<PacketRolesRaw> {
    if bytes.len() < 6 {
        return Err(ParseError::Packet(
            "space packet shorter than 6 octets".into(),
        ));
    }
    let mut apid = None;
    let mut seq_flags = None;
    let mut seq_count = None;
    let mut packet_length = None;
    let mut sec_header_flag = None;
    for pf in &pkt_spec.primary_header {
        let raw = read_bits(bytes, pf.field.offset, pf.field.bits)?;
        match pf.role {
            Some(PacketRole::Apid) => apid = Some(raw as u16),
            Some(PacketRole::SeqFlags) => seq_flags = Some(raw as u8),
            Some(PacketRole::SeqCount) => seq_count = Some(raw as u16),
            Some(PacketRole::PacketLength) => packet_length = Some(raw as u16),
            Some(PacketRole::SecHeaderFlag) => sec_header_flag = Some(raw as u8),
            _ => {}
        }
    }
    Ok(PacketRolesRaw {
        apid: apid.ok_or_else(|| ParseError::Spec("missing APID role".into()))?,
        seq_flags: seq_flags.unwrap_or(3),
        seq_count: seq_count.unwrap_or(0),
        packet_length: packet_length.unwrap_or(0),
        sec_header_flag: sec_header_flag.unwrap_or(0),
    })
}

fn build_packet(
    pkt_spec: &PacketSpec,
    bytes: &[u8],
    frame: &mut DecodedFrame,
) -> Result<DecodedPacket> {
    let roles = parse_packet_roles(pkt_spec, bytes)?;
    let mut packet = DecodedPacket {
        apid: roles.apid,
        seq_flags: roles.seq_flags,
        seq_count: roles.seq_count,
        packet_length: roles.packet_length,
        data_octets: bytes.len(),
        ..Default::default()
    };

    for pf in &pkt_spec.primary_header {
        let raw = read_bits(bytes, pf.field.offset, pf.field.bits)?;
        packet
            .primary_fields
            .push((pf.field.name.clone(), Field::from_spec(&pf.field, raw)));
    }

    let mut payload_start = 6usize;
    if let Some(apid_spec) = pkt_spec.secondary_by_apid.get(&roles.apid) {
        packet.label = Some(apid_spec.label.clone());
        if roles.sec_header_flag == 1 {
            let sh_len = apid_spec.secondary_header_octets;
            if 6 + sh_len > bytes.len() {
                return Err(ParseError::Packet(format!(
                    "APID {} secondary header of {sh_len} octets does not fit",
                    roles.apid
                )));
            }
            let region = &bytes[6..6 + sh_len];
            for fs in &apid_spec.fields {
                let raw = read_bits(region, fs.offset, fs.bits)?;
                let field = Field::from_spec(fs, raw);
                let flat = format!("pkt_{}_{}", sanitize(&apid_spec.label), fs.name);
                frame.values.insert(flat.clone(), field.comparable(fs.bits));
                frame.field_widths.insert(flat, fs.bits);
                packet.secondary_fields.push((fs.name.clone(), field));
            }
            payload_start = 6 + sh_len;
        }
    }

    if bytes.len() > payload_start {
        packet.payload_hex = crate::bits::to_hex(&bytes[payload_start..]);
    }
    Ok(packet)
}

fn virtual_channel(spec: &FrameSpec, frame: &DecodedFrame) -> Option<u16> {
    // Virtual channel id is conventionally declared as field `vcid`; allow
    // several common aliases.
    for key in ["vcid", "virtual_channel_id", "spacecraft_vcid"] {
        if let Some(value) = frame.get(key) {
            return value.as_u64().map(|v| v as u16);
        }
    }
    let _ = spec;
    None
}

fn sanitize(input: &str) -> String {
    input
        .trim()
        .to_ascii_lowercase()
        .replace(|c: char| !c.is_ascii_alphanumeric(), "_")
}

/// Read-only helper retained for potential external use.
pub fn field_map(frame: &DecodedFrame) -> &BTreeMap<String, u32> {
    &frame.field_widths
}
