use frame_parser::builder::{build_tm_frame, FrameInput, PacketInput};
use frame_parser::decoder::decode_stream;
use frame_parser::spec::FrameSpec;

use std::path::Path;

fn example_spec() -> FrameSpec {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .unwrap()
        .to_path_buf();
    FrameSpec::from_json_file(&root.join("examples/tm_frame_spec.json")).unwrap()
}

#[test]
fn loads_and_validates_spec() {
    let spec = example_spec();
    assert_eq!(spec.name, "ExampleSat-1 TM Transfer Frame");
    assert_eq!(spec.frame_length_octets, 1115);
    assert!(spec.secondary_header.is_some());
    assert!(spec.packet.is_some());
}

#[test]
fn builds_and_decodes_nominal_frame() {
    let spec = example_spec();
    let sec = spec.secondary_header.as_ref().unwrap();
    let apid_spec = spec
        .packet
        .as_ref()
        .unwrap()
        .secondary_by_apid
        .get(&26)
        .unwrap();
    assert_eq!(apid_spec.label, "eps_hk");

    // Raw values: 28.0 V bus -> 280 at 0.1 scale; 120000 ms timestamp.
    let assignments: Vec<_> = sec
        .fields
        .iter()
        .map(|f| {
            let value = match f.name.as_str() {
                "sh_timestamp" => 120_000u64,
                "sh_sc_mode" => 2,
                "sh_eps_mode" => 2,
                "sh_batt_voltage" => 2_800, // 28.0 V at 0.01
                "sh_batt_current" => 2_000,
                "sh_batt_temp" => 20,
                "sh_bus_voltage" => 280,
                "sh_xponder_status" => 2,
                "sh_heater_status" => 0,
                "sh_solar_current" => 450,
                _ => 0,
            };
            (f, value)
        })
        .collect();

    // Secondary header (8 octets):
    //   [0] battery_temp = 20
    //   [1] power_mode=2(SUN), heater=0, charger_fault=0, 2b spare
    //   [2..4] solar current = 450 (4.50 A at scale 0.01)
    //   [4..6] load current = 100 (1.00 A)
    //   [6..8] bus voltage = 28000 (28.000 V at scale 0.001)
    // Followed by 6 octets of arbitrary payload.
    let mut eps_data = vec![20u8, 0b0010_0000, 0x01, 0xC2, 0x00, 0x64, 0x6D, 0x60];
    eps_data.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]);
    let packets = vec![PacketInput::unsegmented(26, 7, eps_data)];

    let input = FrameInput {
        spacecraft_id: 0x123,
        vcid: 0,
        master_frame_count: 10,
        vc_frame_count: 5,
        secondary_header_fields: assignments,
        packets,
        include_ocf: false,
        ocf_word: 0,
    };

    let bytes = build_tm_frame(&spec, &input).unwrap();
    // 4 octet ASM + 1115 octet frame.
    assert_eq!(bytes.len(), 1119);
    assert_eq!(&bytes[..4], &[0x1A, 0xCF, 0xFC, 0x1D]);

    let frames = decode_stream(&spec, &bytes).unwrap();
    assert_eq!(frames.len(), 1);
    let frame = &frames[0];
    assert_eq!(frame.get("spacecraft_id").unwrap().as_u64(), Some(0x123));
    assert_eq!(frame.get("sh_sc_mode").unwrap(), "NOMINAL");
    assert_eq!(frame.get("sh_eps_mode").unwrap(), "EPS_SUN");
    assert_eq!(frame.get("sh_batt_temp").unwrap().as_i64(), Some(20));

    let real: Vec<_> = frame
        .packets
        .iter()
        .filter(|p| p.apid != spec.packet.as_ref().unwrap().idle_apid)
        .collect();
    assert_eq!(real.len(), 1);
    let eps = real[0];
    assert_eq!(eps.apid, 26);
    assert_eq!(eps.seq_count, 7);
    assert_eq!(eps.data_octets, 20);
    assert_eq!(frame.get("pkt_eps_hk_power_mode").unwrap(), "PWR_SUN");
    assert!(frame.warnings.is_empty(), "warnings: {:?}", frame.warnings);
}

#[test]
fn idle_only_frame_has_no_packets() {
    let spec = example_spec();
    let input = FrameInput::nominal(0x123, 0, 0, 0);
    let bytes = build_tm_frame(&spec, &input).unwrap();
    let frames = decode_stream(&spec, &bytes).unwrap();
    assert!(frames[0].packets.is_empty());
}

/// A single space packet split over two TM frames: frame 1 holds the first
/// segment starting at the FHP; frame 2 uses FHP = 0x7FF ("no packet start")
/// and appends the remaining octets.
#[test]
fn reassembles_packet_spanning_frames() {
    use frame_parser::bits::write_bits;
    use frame_parser::builder::PacketInput;

    let spec = example_spec();
    let frame_len = spec.frame_length_octets;
    let asm = vec![0x1A, 0xCF, 0xFC, 0x1Du8];
    let data_start = 6usize;
    let data_cap = frame_len - data_start; // no secondary header, no OCF

    // Payload deliberately larger than one data field so the packet spans
    // both frames.
    let payload_len = data_cap + 200;
    let packet = PacketInput::first(26, 9, vec![0xA5u8; payload_len]).to_wire();
    assert_eq!(packet.len(), 6 + payload_len);
    let split = data_cap; // frame 1 fills entirely with the first segment
    assert!(split < packet.len());

    let make_frame = |vc_count: u8, fhp: u16| {
        let mut frame = vec![0u8; frame_len];
        frame[2] = 0; // master frame count
        frame[3] = vc_count;
        write_bits(&mut frame[..6], 32, 1, 0); // sync flag = PACKET
        write_bits(&mut frame[..6], spec.fhp_bit, 11, fhp as u64);
        frame
    };

    // Frame 1: first 300 octets of the packet starting at data offset 0.
    let mut frame1 = make_frame(0, 0);
    frame1[data_start..data_start + split].copy_from_slice(&packet[..split]);

    // Frame 2: continuation only.
    let remainder = &packet[split..];
    assert!(remainder.len() <= data_cap);
    let mut frame2 = make_frame(1, 0x7FF);
    frame2[data_start..data_start + remainder.len()].copy_from_slice(remainder);
    for b in &mut frame2[data_start + remainder.len()..] {
        *b = spec.fill_byte;
    }

    let mut stream = asm.clone();
    stream.extend_from_slice(&frame1);
    stream.extend_from_slice(&asm);
    stream.extend_from_slice(&frame2);

    let frames = decode_stream(&spec, &stream).unwrap();
    assert_eq!(frames.len(), 2);
    assert!(frames[0].packets.is_empty());
    assert!(frames[0].warnings.is_empty(), "{:?}", frames[0].warnings);
    assert_eq!(frames[1].packets.len(), 1);
    assert!(frames[1].warnings.is_empty(), "{:?}", frames[1].warnings);
    let out = &frames[1].packets[0];
    assert_eq!(out.apid, 26);
    assert_eq!(out.seq_count, 9);
    assert_eq!(out.data_octets, packet.len());
    assert_eq!(out.payload_hex.len(), payload_len * 2);
}
