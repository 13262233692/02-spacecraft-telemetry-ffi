//! Fault-tree rule definitions loaded from YAML.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Severity ordered from healthy to catastrophic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    #[default]
    Info = 0,
    Warning = 1,
    Major = 2,
    Critical = 3,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Warning => "WARNING",
            Severity::Major => "MAJOR",
            Severity::Critical => "CRITICAL",
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    /// True when every child is true.
    And,
    /// True when at least one child is true.
    Or,
    /// True when the single child is false (and vice versa).
    Not,
    /// True when at least `threshold` children are true.
    AtLeast,
}

/// A leaf condition expressed in the mini expression language, e.g.
/// `sh_batt_temp < 0` or `sh_eps_mode == "EPS_BATT"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    pub expr: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// A node of the fault tree: either a logical gate over child nodes or a
/// single expression leaf.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "gate", default)]
    pub gate: Option<GateKind>,
    #[serde(default)]
    pub threshold: Option<u32>,
    #[serde(default)]
    pub children: Vec<Node>,
    #[serde(rename = "expr", default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub subsystem: String,
    pub title: String,
    #[serde(default)]
    pub severity: Severity,
    pub tree: Node,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default)]
    pub recommendation: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleSet {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl RuleSet {
    pub fn from_yaml(yaml: &str) -> crate::error::Result<Self> {
        let set: RuleSet = serde_yaml::from_str(yaml)?;
        set.validate()?;
        Ok(set)
    }

    pub fn from_yaml_file(path: &std::path::Path) -> crate::error::Result<Self> {
        Self::from_yaml(&std::fs::read_to_string(path)?)
    }

    fn validate(&self) -> crate::error::Result<()> {
        let mut seen = BTreeMap::new();
        for rule in &self.rules {
            if let Some(prev) = seen.insert(rule.id.clone(), rule.subsystem.clone()) {
                return Err(crate::error::RuleError::Definition(format!(
                    "duplicate rule id `{}` (subsystems {prev} and {})",
                    rule.id, rule.subsystem
                )));
            }
            validate_node(&rule.tree, &rule.id)?;
        }
        Ok(())
    }
}

fn validate_node(node: &Node, rule_id: &str) -> crate::error::Result<()> {
    if node.condition.is_some() == node.gate.is_some() {
        return Err(crate::error::RuleError::Definition(format!(
            "rule `{rule_id}`: every fault-tree node must be exactly one of gate/expr"
        )));
    }
    if let Some(gate) = node.gate {
        let expected = match gate {
            GateKind::Not => 1,
            GateKind::AtLeast => {
                let threshold = node.threshold.ok_or_else(|| {
                    crate::error::RuleError::Definition(format!(
                        "rule `{rule_id}`: at_least gate requires a threshold"
                    ))
                })?;
                if threshold == 0 || threshold as usize > node.children.len() {
                    return Err(crate::error::RuleError::Definition(format!(
                        "rule `{rule_id}`: threshold {threshold} out of range for {} children",
                        node.children.len()
                    )));
                }
                node.children.len()
            }
            GateKind::And | GateKind::Or => node.children.len(),
        };
        if matches!(gate, GateKind::Not) && node.children.len() != expected {
            return Err(crate::error::RuleError::Definition(format!(
                "rule `{rule_id}`: not gate must have exactly one child"
            )));
        }
        if node.children.is_empty() {
            return Err(crate::error::RuleError::Definition(format!(
                "rule `{rule_id}`: gate must have at least one child"
            )));
        }
        for child in &node.children {
            validate_node(child, rule_id)?;
        }
    }
    Ok(())
}
