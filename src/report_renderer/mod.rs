//! ANSI 彩色报告渲染。

use colored::Colorize;

use crate::frame_parser::ParsedFrame;
use crate::rule_engine::{Health, InferenceResult, Severity};

fn health_badge(health: Health) -> colored::ColoredString {
    match health {
        Health::Nominal => " NOMINAL ".on_green().black().bold(),
        Health::Warning => " WARNING ".on_yellow().black().bold(),
        Health::Critical => " CRITICAL ".on_red().white().bold(),
    }
}

fn severity_tag(severity: Severity) -> colored::ColoredString {
    match severity {
        Severity::Info => "INFO".cyan(),
        Severity::Warning => "WARN".yellow().bold(),
        Severity::Critical => "CRIT".red().bold(),
    }
}

fn separator() {
    println!("{}", "─".repeat(64).bright_black());
}

/// 渲染 `parse` 子命令输出：主帧头 + 全部字段表。
pub fn render_parse(def_name: &str, frame: &ParsedFrame) {
    let h = &frame.header;
    println!(
        "{}",
        format!("══ CCSDS TM 帧解析结果 :: {def_name} ══")
            .bright_cyan()
            .bold()
    );
    println!("{}", "[主帧头 Primary Header]".bright_white().bold());
    println!(
        "  版本号 {}  航天器ID {}  虚拟信道 {}  OCF {}",
        h.version.to_string().cyan(),
        format!("0x{:03X}", h.spacecraft_id).cyan(),
        h.virtual_channel_id.to_string().cyan(),
        if h.ocf_flag { "是".green() } else { "否".bright_black() },
    );
    println!(
        "  主信道计数 {}  虚拟信道计数 {}  副帧头 {}  首包头指针 {}",
        h.master_channel_frame_count.to_string().cyan(),
        h.virtual_channel_frame_count.to_string().cyan(),
        if h.secondary_header_flag {
            "有".green()
        } else {
            "无".bright_black()
        },
        format!("0x{:03X}", h.first_header_pointer).cyan(),
    );
    separator();
    println!("{}", "[数据域字段]".bright_white().bold());
    println!(
        "  {:<22} {:<18} {}",
        "字段".bright_black(),
        "值".bright_black(),
        "单位".bright_black()
    );
    for (name, value, unit) in &frame.fields {
        println!(
            "  {:<22} {:<18} {}",
            name.white(),
            value.to_string().bright_yellow(),
            unit.clone().unwrap_or_default().bright_black()
        );
    }
    separator();
}

/// 渲染 `infer` 子命令输出：故障结论 + 分系统状态。
pub fn render_infer(result: &InferenceResult) {
    println!("{}", "══ 故障推理报告 ══".bright_cyan().bold());
    println!("整星状态: {}", health_badge(result.overall));
    separator();
    println!("{}", "[分系统状态]".bright_white().bold());
    for (subsystem, health) in &result.subsystem_status {
        println!("  {:<14} {}", subsystem.white(), health_badge(*health));
    }
    separator();
    if result.findings.is_empty() {
        println!("{}", "✔ 未命中任何故障规则，各分系统运行正常。".green());
    } else {
        println!("{}", "[命中故障规则]".bright_white().bold());
        for f in &result.findings {
            println!(
                "  {} [{}] {} :: {}",
                severity_tag(f.severity),
                f.rule_id.bright_black(),
                f.subsystem.magenta(),
                f.message.white()
            );
        }
    }
    separator();
}

/// 单字段差异行。
pub struct DiffRow {
    pub name: String,
    pub old: String,
    pub new: String,
}

/// 渲染 `diff` 子命令输出。
pub fn render_diff(path_a: &str, path_b: &str, rows: &[DiffRow]) {
    println!("{}", "══ 帧差异对比 ══".bright_cyan().bold());
    println!("  A: {}", path_a.bright_black());
    println!("  B: {}", path_b.bright_black());
    separator();
    if rows.is_empty() {
        println!("{}", "✔ 两帧所有字段一致。".green());
    } else {
        println!(
            "  {:<22} {:<18} {:<18}",
            "字段".bright_black(),
            "A".bright_black(),
            "B".bright_black()
        );
        for row in rows {
            println!(
                "  {:<22} {:<18} {:<18}",
                row.name.white(),
                row.old.yellow(),
                row.new.bright_red().bold()
            );
        }
        separator();
        println!(
            "{}",
            format!("共 {} 个字段发生变化", rows.len()).bright_yellow()
        );
    }
    separator();
}
