//! Fault-tree inference over decoded telemetry frames.

use std::collections::BTreeMap;

use frame_parser::DecodedFrame;
use serde::Serialize;

use crate::error::Result;
use crate::expr::eval_bool;
use crate::model::{GateKind, Node, Rule, RuleSet, Severity};

/// Outcome of evaluating one fault-tree node.
#[derive(Debug, Clone, Serialize)]
pub struct TraceNode {
    pub name: String,
    pub kind: &'static str,
    pub triggered: bool,
    pub detail: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TraceNode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuleResult {
    pub rule_id: String,
    pub subsystem: String,
    pub title: String,
    pub severity: Severity,
    pub triggered: bool,
    pub trace: TraceNode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum OverallStatus {
    Nominal = 0,
    Info = 1,
    Warning = 2,
    Major = 3,
    Critical = 4,
}

impl OverallStatus {
    pub fn label(self) -> &'static str {
        match self {
            OverallStatus::Nominal => "NOMINAL",
            OverallStatus::Info => "INFO",
            OverallStatus::Warning => "WARNING",
            OverallStatus::Major => "MAJOR",
            OverallStatus::Critical => "CRITICAL",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SubsystemStatus {
    pub subsystem: String,
    pub status: OverallStatus,
    /// Ids of the triggered rules, highest severity first.
    pub triggered_rules: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InferenceReport {
    pub frame_index: usize,
    pub overall_status: OverallStatus,
    pub subsystems: Vec<SubsystemStatus>,
    pub results: Vec<RuleResult>,
}

/// Evaluate every rule against a single frame.
pub fn infer_frame(rules: &RuleSet, frame: &DecodedFrame) -> Result<InferenceReport> {
    let mut results = Vec::with_capacity(rules.rules.len());
    for rule in &rules.rules {
        let trace = eval_node(rule, &rule.tree, &frame.values)?;
        let triggered = trace.triggered;
        results.push(RuleResult {
            rule_id: rule.id.clone(),
            subsystem: rule.subsystem.clone(),
            title: rule.title.clone(),
            severity: rule.severity,
            triggered,
            trace,
            rationale: rule.rationale.clone(),
            recommendation: rule.recommendation.clone(),
        });
    }

    // Aggregate by subsystem.
    let mut grouped: BTreeMap<String, SubsystemStatus> = BTreeMap::new();
    for result in &results {
        let entry = grouped
            .entry(result.subsystem.clone())
            .or_insert_with(|| SubsystemStatus {
                subsystem: result.subsystem.clone(),
                status: OverallStatus::Nominal,
                triggered_rules: Vec::new(),
            });
        if result.triggered {
            entry.triggered_rules.push(result.rule_id.clone());
            let candidate = match result.severity {
                Severity::Info => OverallStatus::Info,
                Severity::Warning => OverallStatus::Warning,
                Severity::Major => OverallStatus::Major,
                Severity::Critical => OverallStatus::Critical,
            };
            if candidate > entry.status {
                entry.status = candidate;
            }
        }
    }
    let mut subsystems: Vec<_> = grouped.into_values().collect();
    subsystems.sort_by(|a, b| b.status.cmp(&a.status).then(a.subsystem.cmp(&b.subsystem)));

    let overall_status = subsystems
        .iter()
        .map(|s| s.status)
        .max()
        .unwrap_or(OverallStatus::Nominal);

    // Highest severity first for readability; nominal rules follow.
    results.sort_by_key(|r| (if r.triggered { 0 } else { 1 }, -(r.severity as i32)));

    Ok(InferenceReport {
        frame_index: frame.index,
        overall_status,
        subsystems,
        results,
    })
}

/// Evaluate a full multi-frame stream, returning one report per frame.
pub fn infer_stream(rules: &RuleSet, frames: &[DecodedFrame]) -> Result<Vec<InferenceReport>> {
    frames
        .iter()
        .map(|frame| infer_frame(rules, frame))
        .collect()
}

fn eval_node(
    _rule: &Rule,
    node: &Node,
    values: &serde_json::Map<String, serde_json::Value>,
) -> Result<TraceNode> {
    if let Some(expr) = &node.condition {
        let triggered = eval_bool(expr, values)?;
        return Ok(TraceNode {
            name: node.name.clone().unwrap_or_else(|| short_expr(expr)),
            kind: "condition",
            triggered,
            detail: format!("{expr} => {triggered}"),
            children: Vec::new(),
        });
    }

    let gate = node.gate.expect("validated node");
    let child_traces = node
        .children
        .iter()
        .map(|child| eval_node(_rule, child, values))
        .collect::<Result<Vec<_>>>()?;
    let fired: Vec<bool> = child_traces.iter().map(|c| c.triggered).collect();
    let fired_count = fired.iter().filter(|v| **v).count() as u32;

    let (triggered, label, detail) = match gate {
        GateKind::And => (
            fired.iter().all(|v| *v),
            "and",
            format!("{} of {} conditions met", fired_count, fired.len()),
        ),
        GateKind::Or => (
            fired.iter().any(|v| *v),
            "or",
            format!("{} of {} conditions met", fired_count, fired.len()),
        ),
        GateKind::Not => (
            !fired[0],
            "not",
            format!(
                "inner condition {}",
                if fired[0] { "true" } else { "false" }
            ),
        ),
        GateKind::AtLeast => {
            let threshold = node.threshold.expect("validated threshold");
            (
                fired_count >= threshold,
                "at_least",
                format!(
                    "{fired_count} of {} conditions met (need {threshold})",
                    fired.len()
                ),
            )
        }
    };

    Ok(TraceNode {
        name: node
            .name
            .clone()
            .unwrap_or_else(|| format!("{} gate", label)),
        kind: "gate",
        triggered,
        detail,
        children: child_traces,
    })
}

fn short_expr(expr: &str) -> String {
    expr.chars().take(48).collect()
}
