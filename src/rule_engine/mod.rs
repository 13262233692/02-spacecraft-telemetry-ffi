//! 故障树规则引擎。
//!
//! 从 YAML 加载规则集，对解析后的遥测帧求值，输出故障结论（Finding）
//! 并汇总各分系统健康状态。

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::frame_parser::{FieldValue, ParsedFrame};

/// 严重等级（按序可比较）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Severity::Info => "INFO",
            Severity::Warning => "WARNING",
            Severity::Critical => "CRITICAL",
        };
        f.write_str(s)
    }
}

/// 比较运算符。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
}

/// 故障树条件节点：叶子比较或 all/any/not 组合。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Condition {
    All {
        all: Vec<Condition>,
    },
    Any {
        any: Vec<Condition>,
    },
    Not {
        not: Box<Condition>,
    },
    Leaf {
        field: String,
        op: Op,
        value: serde_yaml::Value,
    },
}

/// 单条故障规则。
#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub id: String,
    /// 所属分系统，如 power / thermal / adcs
    pub subsystem: String,
    pub severity: Severity,
    /// 命中时输出的描述信息
    pub message: String,
    pub when: Condition,
}

/// YAML 规则文件顶层结构。
#[derive(Debug, Clone, Deserialize)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
}

impl RuleSet {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取规则文件失败: {}", path.display()))?;
        let set: RuleSet = serde_yaml::from_str(&text)
            .with_context(|| format!("解析规则 YAML 失败: {}", path.display()))?;
        Ok(set)
    }
}

/// 一条命中的故障结论。
#[derive(Debug, Clone)]
pub struct Finding {
    pub rule_id: String,
    pub subsystem: String,
    pub severity: Severity,
    pub message: String,
}

/// 分系统健康状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Health {
    Nominal,
    Warning,
    Critical,
}

impl fmt::Display for Health {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Health::Nominal => "NOMINAL",
            Health::Warning => "WARNING",
            Health::Critical => "CRITICAL",
        };
        f.write_str(s)
    }
}

/// 推理结果：命中列表 + 各分系统状态 + 整星状态。
#[derive(Debug)]
pub struct InferenceResult {
    pub findings: Vec<Finding>,
    /// 分系统 -> 状态（按名称排序）
    pub subsystem_status: BTreeMap<String, Health>,
    pub overall: Health,
}

fn yaml_to_f64(v: &serde_yaml::Value) -> Option<f64> {
    match v {
        serde_yaml::Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

fn compare_leaf(field_value: &FieldValue, op: Op, expected: &serde_yaml::Value) -> bool {
    // 字符串期望值：优先与枚举标签比较
    if let serde_yaml::Value::String(s) = expected {
        if let Some(label) = field_value.as_label() {
            return match op {
                Op::Eq => label == s,
                Op::Ne => label != s,
                _ => false,
            };
        }
    }
    match op {
        Op::In => {
            if let serde_yaml::Value::Sequence(items) = expected {
                items
                    .iter()
                    .any(|item| compare_leaf(field_value, Op::Eq, item))
            } else {
                false
            }
        }
        _ => {
            let (Some(actual), Some(want)) = (field_value.as_f64(), yaml_to_f64(expected))
            else {
                return false;
            };
            match op {
                Op::Eq => (actual - want).abs() < f64::EPSILON,
                Op::Ne => (actual - want).abs() >= f64::EPSILON,
                Op::Lt => actual < want,
                Op::Le => actual <= want,
                Op::Gt => actual > want,
                Op::Ge => actual >= want,
                Op::In => unreachable!(),
            }
        }
    }
}

fn eval_condition(cond: &Condition, frame: &ParsedFrame) -> bool {
    match cond {
        Condition::All { all } => all.iter().all(|c| eval_condition(c, frame)),
        Condition::Any { any } => any.iter().any(|c| eval_condition(c, frame)),
        Condition::Not { not } => !eval_condition(not, frame),
        Condition::Leaf { field, op, value } => match frame.get(field) {
            Some(fv) => compare_leaf(fv, *op, value),
            None => false,
        },
    }
}

/// 对一帧执行全部规则，汇总推理结果。
pub fn infer(rules: &RuleSet, frame: &ParsedFrame) -> InferenceResult {
    let mut findings = Vec::new();
    let mut subsystem_status: BTreeMap<String, Health> = BTreeMap::new();

    for rule in &rules.rules {
        subsystem_status
            .entry(rule.subsystem.clone())
            .or_insert(Health::Nominal);
        if eval_condition(&rule.when, frame) {
            let health = match rule.severity {
                Severity::Info => Health::Nominal,
                Severity::Warning => Health::Warning,
                Severity::Critical => Health::Critical,
            };
            let entry = subsystem_status.get_mut(&rule.subsystem).unwrap();
            if health > *entry {
                *entry = health;
            }
            findings.push(Finding {
                rule_id: rule.id.clone(),
                subsystem: rule.subsystem.clone(),
                severity: rule.severity,
                message: rule.message.clone(),
            });
        }
    }

    let overall = subsystem_status
        .values()
        .copied()
        .max()
        .unwrap_or(Health::Nominal);

    InferenceResult {
        findings,
        subsystem_status,
        overall,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_parser::{FieldValue, PrimaryHeader};

    fn frame_with(fields: Vec<(&str, FieldValue)>) -> ParsedFrame {
        ParsedFrame {
            header: PrimaryHeader {
                version: 0,
                spacecraft_id: 1,
                virtual_channel_id: 0,
                ocf_flag: false,
                master_channel_frame_count: 0,
                virtual_channel_frame_count: 0,
                secondary_header_flag: false,
                sync_flag: false,
                packet_order_flag: false,
                segment_length_id: 0,
                first_header_pointer: 0,
            },
            fields: fields
                .into_iter()
                .map(|(n, v)| (n.to_string(), v, None))
                .collect(),
        }
    }

    fn ruleset(yaml: &str) -> RuleSet {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn leaf_numeric_and_enum() {
        let rs = ruleset(
            r#"
rules:
  - id: R1
    subsystem: power
    severity: critical
    message: low voltage
    when: { field: v, op: lt, value: 24.0 }
  - id: R2
    subsystem: mode
    severity: warning
    message: safe mode
    when: { field: m, op: eq, value: "SAFE" }
"#,
        );
        let frame = frame_with(vec![
            ("v", FieldValue::Float(23.5)),
            (
                "m",
                FieldValue::Enum {
                    raw: 3,
                    label: "SAFE".into(),
                },
            ),
        ]);
        let result = infer(&rs, &frame);
        assert_eq!(result.findings.len(), 2);
        assert_eq!(result.overall, Health::Critical);
        assert_eq!(result.subsystem_status["power"], Health::Critical);
        assert_eq!(result.subsystem_status["mode"], Health::Warning);
    }

    #[test]
    fn composite_all_any_not() {
        let rs = ruleset(
            r#"
rules:
  - id: C1
    subsystem: thermal
    severity: warning
    message: hot and heater on
    when:
      all:
        - { field: t, op: gt, value: 45 }
        - not: { field: h, op: eq, value: 0 }
  - id: C2
    subsystem: obc
    severity: info
    message: in list
    when: { field: e, op: in, value: [1, 2, 3] }
"#,
        );
        let frame = frame_with(vec![
            ("t", FieldValue::Float(50.0)),
            ("h", FieldValue::UInt(1)),
            ("e", FieldValue::UInt(2)),
        ]);
        let result = infer(&rs, &frame);
        assert_eq!(result.findings.len(), 2);
        assert_eq!(result.overall, Health::Warning);
    }

    #[test]
    fn nominal_when_no_hit() {
        let rs = ruleset(
            r#"
rules:
  - id: R1
    subsystem: power
    severity: critical
    message: low voltage
    when: { field: v, op: lt, value: 24.0 }
"#,
        );
        let frame = frame_with(vec![("v", FieldValue::Float(28.0))]);
        let result = infer(&rs, &frame);
        assert!(result.findings.is_empty());
        assert_eq!(result.overall, Health::Nominal);
    }
}
