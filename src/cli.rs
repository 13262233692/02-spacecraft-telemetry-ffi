//! 命令行接口：parse / infer / diff 三个子命令。

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::frame_parser::{parse_frame, FrameDefinition};
use crate::report_renderer::{self, DiffRow};
use crate::rule_engine::{self, RuleSet};

#[derive(Parser)]
#[command(
    name = "ccsds-tm",
    version,
    about = "CCSDS 遥测帧离线解析与故障推理工具"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 解析单帧并打印全部字段
    Parse {
        /// 二进制帧文件
        #[arg(short, long)]
        frame: PathBuf,
        /// JSON 帧结构定义
        #[arg(short, long)]
        def: PathBuf,
    },
    /// 解析单帧并按 YAML 故障树规则推理分系统状态
    Infer {
        /// 二进制帧文件
        #[arg(short, long)]
        frame: PathBuf,
        /// JSON 帧结构定义
        #[arg(short, long)]
        def: PathBuf,
        /// YAML 故障规则文件
        #[arg(short, long)]
        rules: PathBuf,
    },
    /// 对比两帧的字段差异
    Diff {
        /// JSON 帧结构定义
        #[arg(short, long)]
        def: PathBuf,
        /// 帧 A
        frame_a: PathBuf,
        /// 帧 B
        frame_b: PathBuf,
    },
}

fn load(def: &PathBuf, frame: &PathBuf) -> Result<(FrameDefinition, crate::frame_parser::ParsedFrame)> {
    let definition = FrameDefinition::load(def)?;
    let bytes = std::fs::read(frame)
        .with_context(|| format!("读取帧文件失败: {}", frame.display()))?;
    let parsed = parse_frame(&definition, &bytes)
        .with_context(|| format!("解析帧失败: {}", frame.display()))?;
    Ok((definition, parsed))
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Parse { frame, def } => {
            let (definition, parsed) = load(&def, &frame)?;
            report_renderer::render_parse(&definition.name, &parsed);
        }
        Command::Infer { frame, def, rules } => {
            let (_, parsed) = load(&def, &frame)?;
            let ruleset = RuleSet::load(&rules)?;
            let result = rule_engine::infer(&ruleset, &parsed);
            report_renderer::render_infer(&result);
        }
        Command::Diff {
            def,
            frame_a,
            frame_b,
        } => {
            let (_, a) = load(&def, &frame_a)?;
            let (_, b) = load(&def, &frame_b)?;
            let mut rows = Vec::new();

            if a.header != b.header {
                rows.push(DiffRow {
                    name: "header.mc_frame_count".into(),
                    old: a.header.master_channel_frame_count.to_string(),
                    new: b.header.master_channel_frame_count.to_string(),
                });
                rows.push(DiffRow {
                    name: "header.vc_frame_count".into(),
                    old: a.header.virtual_channel_frame_count.to_string(),
                    new: b.header.virtual_channel_frame_count.to_string(),
                });
            }
            for (name, va, _) in &a.fields {
                if let Some(vb) = b.get(name) {
                    if va != vb {
                        rows.push(DiffRow {
                            name: name.clone(),
                            old: va.to_string(),
                            new: vb.to_string(),
                        });
                    }
                }
            }
            report_renderer::render_diff(
                &frame_a.display().to_string(),
                &frame_b.display().to_string(),
                &rows,
            );
        }
    }
    Ok(())
}
