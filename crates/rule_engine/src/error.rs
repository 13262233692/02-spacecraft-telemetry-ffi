use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuleError {
    #[error("invalid rule set YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("rule definition error: {0}")]
    Definition(String),
    #[error("expression error: {0}")]
    Expression(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, RuleError>;
