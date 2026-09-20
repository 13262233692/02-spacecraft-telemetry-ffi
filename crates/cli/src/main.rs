//! tmctl — offline CCSDS TM telemetry frame parser and fault inference CLI.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use frame_parser::builder::{build_tm_frame, FrameInput, PacketInput};
use frame_parser::decoder::decode_stream;
use frame_parser::spec::FrameSpec;
use rule_engine::engine::infer_stream;
use rule_engine::RuleSet;

#[derive(Parser)]
#[command(
    name = "tmctl",
    version,
    about = "CCSDS TM telemetry frame parser & fault-tree inference",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse binary frame file(s) and print decoded fields.
    Parse(ParseArgs),
    /// Parse frames and evaluate YAML fault-tree rules.
    Infer(InferArgs),
    /// Compare two frames (or two positions inside one file) field by field.
    Diff(DiffArgs),
    /// Generate a ready-to-use nominal/anomaly sample set.
    GenSample(GenArgs),
}

#[derive(Args)]
struct CommonIo {
    /// Frame structure definition JSON.
    #[arg(
        short = 's',
        long = "spec",
        default_value = "examples/tm_frame_spec.json"
    )]
    spec: PathBuf,
    /// Binary frame input file.
    input: PathBuf,
    /// Emit machine-readable JSON instead of the colored report.
    #[arg(long)]
    json: bool,
    /// Disable ANSI colors even on a TTY.
    #[arg(long = "no-color")]
    no_color: bool,
    /// Force ANSI colors even when stdout is redirected.
    #[arg(long)]
    color: bool,
    /// Limit parsing to the first N frames.
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Args)]
struct ParseArgs {
    #[command(flatten)]
    io: CommonIo,
}

#[derive(Args)]
struct InferArgs {
    #[command(flatten)]
    io: CommonIo,
    /// Fault-tree rules YAML.
    #[arg(
        short = 'r',
        long = "rules",
        default_value = "examples/fault_rules.yaml"
    )]
    rules: PathBuf,
}

#[derive(Args)]
struct DiffArgs {
    /// Frame structure definition JSON.
    #[arg(
        short = 's',
        long = "spec",
        default_value = "examples/tm_frame_spec.json"
    )]
    spec: PathBuf,
    /// Left frame file.
    left: PathBuf,
    /// Right frame file.
    #[arg(long)]
    right: Option<PathBuf>,
    /// Left frame index (zero based).
    #[arg(long)]
    left_index: Option<usize>,
    /// Right frame index. Defaults to 1 within one file, otherwise 0.
    #[arg(long)]
    right_index: Option<usize>,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    no_color: bool,
    #[arg(long)]
    color: bool,
}

#[derive(Args)]
struct GenArgs {
    /// Output directory for sample frames and copied definitions.
    #[arg(short = 'o', long = "out", default_value = "examples/generated")]
    out: PathBuf,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Parse(args) => {
            configure_color(args.io.color, args.io.no_color);
            let spec = FrameSpec::from_json_file(&args.io.spec).map_err(|e| e.to_string())?;
            let bytes = std::fs::read(&args.io.input).map_err(|e| e.to_string())?;
            let mut frames = decode_stream(&spec, &bytes).map_err(|e| e.to_string())?;
            if let Some(limit) = args.io.limit {
                frames.truncate(limit);
            }
            if args.io.json {
                let payload = serde_json::to_string_pretty(&frames_json(&frames))
                    .map_err(|e| e.to_string())?;
                println!("{payload}");
            } else {
                print!("{}", report_renderer::render_parse_report(&spec, &frames));
            }
        }
        Command::Infer(args) => {
            configure_color(args.io.color, args.io.no_color);
            let spec = FrameSpec::from_json_file(&args.io.spec).map_err(|e| e.to_string())?;
            let rules = RuleSet::from_yaml_file(&args.rules).map_err(|e| e.to_string())?;
            let bytes = std::fs::read(&args.io.input).map_err(|e| e.to_string())?;
            let mut frames = decode_stream(&spec, &bytes).map_err(|e| e.to_string())?;
            if let Some(limit) = args.io.limit {
                frames.truncate(limit);
            }
            let reports = infer_stream(&rules, &frames).map_err(|e| e.to_string())?;
            if args.io.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&reports).map_err(|e| e.to_string())?
                );
            } else {
                print!("{}", report_renderer::render_inference_reports(&reports));
            }
        }
        Command::Diff(args) => {
            configure_color(args.color, args.no_color);
            let spec = FrameSpec::from_json_file(&args.spec).map_err(|e| e.to_string())?;
            let left_bytes = std::fs::read(&args.left).map_err(|e| e.to_string())?;
            let left_frames = decode_stream(&spec, &left_bytes).map_err(|e| e.to_string())?;
            let (right_path, right_bytes) = match &args.right {
                Some(path) => (
                    path.clone(),
                    std::fs::read(path).map_err(|e| e.to_string())?,
                ),
                None => (args.left.clone(), left_bytes.clone()),
            };
            let right_frames = decode_stream(&spec, &right_bytes).map_err(|e| e.to_string())?;
            let left_index = args.left_index.unwrap_or(0);
            let right_index =
                args.right_index
                    .unwrap_or_else(|| if args.right.is_some() { 0 } else { 1 });
            let left = pick_frame(&left_frames, left_index, &args.left)?;
            let right = pick_frame(&right_frames, right_index, &right_path)?;
            if args.json {
                let diff = json_diff(left, right);
                println!(
                    "{}",
                    serde_json::to_string_pretty(&diff).map_err(|e| e.to_string())?
                );
            } else {
                print!("{}", report_renderer::render_diff(left, right));
            }
        }
        Command::GenSample(args) => generate_samples(&args.out)?,
    }
    Ok(())
}

fn configure_color(force: bool, disable: bool) {
    if disable {
        report_renderer::set_color(false);
    } else if force {
        report_renderer::set_color(true);
    }
}

fn pick_frame<'a>(
    frames: &'a [frame_parser::DecodedFrame],
    index: usize,
    path: &std::path::Path,
) -> Result<&'a frame_parser::DecodedFrame, String> {
    frames.get(index).ok_or_else(|| {
        format!(
            "{} contains only {} frame(s), cannot select #{index}",
            path.display(),
            frames.len()
        )
    })
}

fn frames_json(frames: &[frame_parser::DecodedFrame]) -> serde_json::Value {
    serde_json::json!({
        "frame_count": frames.len(),
        "frames": frames.iter().map(|f| serde_json::json!({
            "index": f.index,
            "file_offset": f.file_offset,
            "frame_octets": f.frame_octets,
            "values": f.values,
            "packets": f.packets.iter().map(|p| serde_json::json!({
                "apid": p.apid,
                "label": p.label,
                "seq_flags": p.seq_flags,
                "seq_count": p.seq_count,
                "packet_length": p.packet_length,
                "data_octets": p.data_octets,
                "payload_hex": p.payload_hex,
            })).collect::<Vec<_>>(),
            "warnings": f.warnings,
        })).collect::<Vec<_>>(),
    })
}

fn json_diff(
    left: &frame_parser::DecodedFrame,
    right: &frame_parser::DecodedFrame,
) -> serde_json::Value {
    let mut changed = serde_json::Map::new();
    let mut keys: Vec<&String> = left.values.keys().collect();
    for key in right.values.keys() {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    for key in keys {
        let l = left.values.get(key);
        let r = right.values.get(key);
        if l != r {
            changed.insert(key.clone(), serde_json::json!({ "left": l, "right": r }));
        }
    }
    serde_json::json!({
        "left_frame": left.index,
        "right_frame": right.index,
        "differences": changed,
    })
}

fn generate_samples(out: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let manifest = std::env::var("CARGO_MANIFEST_DIR").ok();
    let base = manifest
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    // When run via cargo the cwd is the workspace root for `--manifest-path`
    // builds too; fall back to known relative locations.
    let candidates = [
        PathBuf::from("examples/tm_frame_spec.json"),
        base.join("../../examples/tm_frame_spec.json"),
    ];
    let spec_path = candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .ok_or_else(|| "cannot locate examples/tm_frame_spec.json".to_string())?;
    let spec = FrameSpec::from_json_file(&spec_path).map_err(|e| e.to_string())?;
    let sec = spec
        .secondary_header
        .as_ref()
        .ok_or_else(|| "sample spec needs a secondary header".to_string())?;

    let make_frame = |values: &[(&str, u64)], packet_temp: i8, fault: bool| {
        let assignments: Vec<_> = sec
            .fields
            .iter()
            .map(|f| {
                let value = values
                    .iter()
                    .find(|(name, _)| *name == f.name)
                    .map(|(_, v)| *v)
                    .unwrap_or(0);
                (f, value)
            })
            .collect();
        let flag_byte: u8 = if fault { 0b0001_0100 } else { 0b0010_0000 };
        let mut sh = vec![
            packet_temp as u8,
            flag_byte,
            0x01,
            0xC2,
            0x00,
            0x64,
            0x6D,
            0x60,
        ];
        sh.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]);
        let packets = vec![PacketInput::unsegmented(26, 0, sh)];
        FrameInput {
            spacecraft_id: 0x123,
            vcid: 0,
            master_frame_count: 0,
            vc_frame_count: 0,
            secondary_header_fields: assignments,
            packets,
            include_ocf: false,
            ocf_word: 0,
        }
    };

    let nominal_values: Vec<(&str, u64)> = vec![
        ("sh_timestamp", 100_000),
        ("sh_sc_mode", 2),
        ("sh_eps_mode", 2),
        ("sh_batt_voltage", 2_800),
        ("sh_batt_current", 2_000),
        ("sh_batt_temp", 20),
        ("sh_bus_voltage", 280),
        ("sh_xponder_status", 2),
        ("sh_heater_status", 0),
        ("sh_solar_current", 450),
    ];
    let anomaly_values: Vec<(&str, u64)> = vec![
        ("sh_timestamp", 100_001),
        ("sh_sc_mode", 0),
        ("sh_eps_mode", 1),
        ("sh_batt_voltage", 2_200),
        ("sh_batt_current", 2_000),
        ("sh_batt_temp", 50),
        ("sh_bus_voltage", 220),
        ("sh_xponder_status", 3),
        ("sh_heater_status", 0),
        ("sh_solar_current", 60),
    ];

    let nominal = build_tm_frame(&spec, &make_frame(&nominal_values, 20, false))
        .map_err(|e| e.to_string())?;
    let anomaly =
        build_tm_frame(&spec, &make_frame(&anomaly_values, 50, true)).map_err(|e| e.to_string())?;

    write_file(&out.join("nominal.tm"), &nominal)?;
    write_file(&out.join("anomaly.tm"), &anomaly)?;
    let mut both = nominal.clone();
    both.extend_from_slice(&anomaly);
    write_file(&out.join("stream.tm"), &both)?;

    println!(
        "Wrote sample frames to {}:",
        out.canonicalize()
            .unwrap_or_else(|_| out.to_path_buf())
            .display()
    );
    println!("  nominal.tm  — healthy spacecraft, no rules trigger");
    println!("  anomaly.tm  — undervoltage / over-temp / transponder fault");
    println!("  stream.tm   — both frames concatenated");
    Ok(())
}

fn write_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())
}
