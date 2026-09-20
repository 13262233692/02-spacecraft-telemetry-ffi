//! Rendering of fault-tree inference results.

use rule_engine::{InferenceReport, OverallStatus, RuleResult, Severity, TraceNode};

use crate::color::{bold, dim, paint, Style};

fn status_style(status: OverallStatus) -> Style {
    match status {
        OverallStatus::Nominal => Style::GreenBold,
        OverallStatus::Info => Style::CyanBold,
        OverallStatus::Warning => Style::YellowBold,
        OverallStatus::Major => Style::RedBold,
        OverallStatus::Critical => Style::RedBold,
    }
}

fn severity_style(severity: Severity) -> Style {
    match severity {
        Severity::Info => Style::Cyan,
        Severity::Warning => Style::Yellow,
        Severity::Major => Style::Red,
        Severity::Critical => Style::RedBold,
    }
}

pub fn render_reports(reports: &[InferenceReport]) -> String {
    let mut out = String::new();
    for report in reports {
        out.push_str(&render_report(report));
        out.push('\n');
    }
    out
}

pub fn render_report(report: &InferenceReport) -> String {
    let mut out = String::new();
    let title = format!("Fault Inference — Frame #{}", report.frame_index);
    out.push_str(&bold(&title));
    out.push('\n');

    let status = paint(
        status_style(report.overall_status),
        report.overall_status.label(),
    );
    out.push_str(&format!("  Overall status: {status}\n"));

    out.push_str("  Subsystems:\n");
    if report.subsystems.is_empty() {
        out.push_str(&format!("    {}\n", dim("none declared")));
    }
    for subsystem in &report.subsystems {
        let style = status_style(subsystem.status);
        let suffix = if subsystem.triggered_rules.is_empty() {
            String::new()
        } else {
            format!(" [{}]", subsystem.triggered_rules.join(", "))
        };
        out.push_str(&format!(
            "    {:<12} {}{suffix}\n",
            paint(Style::White, &subsystem.subsystem),
            paint(style, subsystem.status.label()),
        ));
    }

    let fired: Vec<&RuleResult> = report.results.iter().filter(|r| r.triggered).collect();
    out.push_str(&format!(
        "  Triggered rules: {}/{}\n",
        fired.len(),
        report.results.len()
    ));
    if fired.is_empty() {
        out.push_str(&format!(
            "    {}\n",
            paint(Style::Green, "All fault-tree conditions evaluate false.")
        ));
    }
    for rule in &fired {
        out.push_str(&render_rule(rule));
    }
    out
}

fn render_rule(rule: &RuleResult) -> String {
    let mut out = String::new();
    let header = format!(
        "  {} {} — {}",
        paint(severity_style(rule.severity), rule.severity.as_str()),
        bold(&rule.rule_id),
        rule.title
    );
    out.push_str(&header);
    out.push('\n');
    out.push_str(&render_trace(&rule.trace, 2));
    if let Some(rationale) = &rule.rationale {
        out.push_str(&format!("    {} {rationale}\n", dim("rationale:")));
    }
    if let Some(rec) = &rule.recommendation {
        out.push_str(&format!(
            "    {} {rec}\n",
            paint(Style::Cyan, "action:    ")
        ));
    }
    out
}

fn render_trace(node: &TraceNode, depth: usize) -> String {
    let mut out = String::new();
    let indent = "  ".repeat(depth);
    let mark = if node.triggered {
        paint(Style::RedBold, "[X]")
    } else {
        paint(Style::Green, "[ ]")
    };
    let kind = match node.kind {
        "gate" => paint(Style::Blue, "gate"),
        _ => paint(Style::Gray, "cond"),
    };
    out.push_str(&format!("{indent}{mark} {kind} {}\n", dim(&node.detail)));
    for child in &node.children {
        out.push_str(&render_trace(child, depth + 1));
    }
    out
}
