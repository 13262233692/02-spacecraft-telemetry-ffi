use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("frame structure definition error: {0}")]
    Spec(String),
    #[error("frame #{index} is truncated: need {need} octets at offset {offset}, have {have}")]
    Truncated {
        index: usize,
        offset: usize,
        need: usize,
        have: usize,
    },
    #[error("field `{field}` could not be decoded: {reason}")]
    Field { field: String, reason: String },
    #[error("packet reassembly error: {0}")]
    Packet(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ParseError>;
