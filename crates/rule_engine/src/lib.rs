//! `rule_engine`: YAML-defined CCSDS telemetry fault-tree inference.

pub mod engine;
pub mod error;
pub mod expr;
pub mod model;

pub use engine::{
    infer_frame, infer_stream, InferenceReport, OverallStatus, RuleResult, SubsystemStatus,
    TraceNode,
};
pub use error::{Result, RuleError};
pub use model::{GateKind, Node, Rule, RuleSet, Severity};
