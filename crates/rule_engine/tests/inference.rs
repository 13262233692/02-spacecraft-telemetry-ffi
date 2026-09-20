use frame_parser::builder::{build_tm_frame, FrameInput, PacketInput};
use frame_parser::decoder::decode_stream;
use frame_parser::spec::FrameSpec;
use rule_engine::{infer_stream, RuleSet};
use std::path::Path;

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn loads_yaml_rules() {
    let rules = RuleSet::from_yaml_file(&root().join("examples/fault_rules.yaml")).unwrap();
    assert!(rules.rules.len() >= 10);
}

#[test]
fn nominal_frame_is_healthy() {
    let spec = FrameSpec::from_json_file(&root().join("examples/tm_frame_spec.json")).unwrap();
    let rules = RuleSet::from_yaml_file(&root().join("examples/fault_rules.yaml")).unwrap();
    let sec = spec.secondary_header.as_ref().unwrap();
    let assignments: Vec<_> = sec
        .fields
        .iter()
        .map(|f| {
            let v = match f.name.as_str() {
                "sh_timestamp" => 100,
                "sh_sc_mode" => 2,
                "sh_eps_mode" => 2,
                "sh_batt_voltage" => 2_800,
                "sh_batt_current" => 2_000,
                "sh_batt_temp" => 20,
                "sh_bus_voltage" => 280,
                "sh_xponder_status" => 2,
                "sh_heater_status" => 0,
                "sh_solar_current" => 450,
                _ => 0,
            };
            (f, v)
        })
        .collect();
    let mut sh = vec![20u8, 0x20, 0x01, 0xC2, 0x00, 0x64, 0x6D, 0x60];
    sh.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
    let input = FrameInput {
        spacecraft_id: 0x123,
        vcid: 0,
        master_frame_count: 1,
        vc_frame_count: 1,
        secondary_header_fields: assignments,
        packets: vec![PacketInput::unsegmented(26, 1, sh)],
        include_ocf: false,
        ocf_word: 0,
    };
    let bytes = build_tm_frame(&spec, &input).unwrap();
    let frames = decode_stream(&spec, &bytes).unwrap();
    let reports = infer_stream(&rules, &frames).unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].overall_status,
        rule_engine::OverallStatus::Nominal
    );
    assert!(reports[0].results.iter().all(|r| !r.triggered));
}

#[test]
fn anomalous_frame_triggers_fault_tree() {
    let spec = FrameSpec::from_json_file(&root().join("examples/tm_frame_spec.json")).unwrap();
    let rules = RuleSet::from_yaml_file(&root().join("examples/fault_rules.yaml")).unwrap();
    let sec = spec.secondary_header.as_ref().unwrap();
    // Battery 22.0 V -> 11000 at scale 0.002; temperature 50 degC; charger
    // fault latched; transponder fault.
    let assignments: Vec<_> = sec
        .fields
        .iter()
        .map(|f| {
            let v = match f.name.as_str() {
                "sh_sc_mode" => 0,
                "sh_eps_mode" => 1,
                "sh_batt_voltage" => 2_200,
                "sh_batt_current" => 2_000,
                "sh_batt_temp" => 50,
                "sh_bus_voltage" => 220,
                "sh_xponder_status" => 3,
                "sh_heater_status" => 0,
                _ => 0,
            };
            (f, v)
        })
        .collect();
    // Packet secondary: temp=50, power_mode=BATT, heater=0, charger_fault=1.
    let mut sh = vec![50u8, 0b0001_0100, 0x01, 0x00, 0x00, 0x64, 0x59, 0xD8];
    sh.extend_from_slice(&[0; 6]);
    let input = FrameInput {
        spacecraft_id: 0x123,
        vcid: 0,
        master_frame_count: 2,
        vc_frame_count: 2,
        secondary_header_fields: assignments,
        packets: vec![PacketInput::unsegmented(26, 2, sh)],
        include_ocf: false,
        ocf_word: 0,
    };
    let bytes = build_tm_frame(&spec, &input).unwrap();
    let frames = decode_stream(&spec, &bytes).unwrap();
    assert!(frames[0].warnings.is_empty(), "{:?}", frames[0].warnings);
    let reports = infer_stream(&rules, &frames).unwrap();
    let report = &reports[0];
    assert_eq!(report.overall_status, rule_engine::OverallStatus::Critical);

    let triggered: Vec<&str> = report
        .results
        .iter()
        .filter(|r| r.triggered)
        .map(|r| r.rule_id.as_str())
        .collect();
    assert!(
        triggered.contains(&"EPS_BATT_UNDERVOLTAGE"),
        "{triggered:?}"
    );
    assert!(triggered.contains(&"THERMAL_BATT_HOT"), "{triggered:?}");
    assert!(triggered.contains(&"TTXPONDER_FAULT"), "{triggered:?}");
    assert!(triggered.contains(&"EPS_CHARGER_FAULT"), "{triggered:?}");
    assert!(triggered.contains(&"SAFE_MODE_UNEXPECTED"), "{triggered:?}");
    assert!(
        triggered.contains(&"COMPOUND_POWER_EMERGENCY"),
        "{triggered:?}"
    );
}
