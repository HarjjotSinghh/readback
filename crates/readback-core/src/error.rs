//! Errors surfaced by the adapters. The pipeline itself is infallible.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("input was not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{engine} output is missing the `{field}` field")]
    MissingField {
        engine: &'static str,
        field: &'static str,
    },
    #[error("{engine} output had no usable transcript")]
    Empty { engine: &'static str },
}

pub type Result<T> = std::result::Result<T, AdapterError>;
