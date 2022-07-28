//! Errors produced by crate logic.

use std::{error, fmt, str::Utf8Error};

/// Kind of a [`ReadError`].
#[derive(Debug)]
#[non_exhaustive]
pub enum ReadErrorKind {
    /// Unexpected end of the input.
    UnexpectedEof,
    /// Error parsing
    Utf8(Utf8Error),
}

impl fmt::Display for ReadErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => formatter.write_str("reached end of input"),
            Self::Utf8(err) => write!(formatter, "{}", err),
        }
    }
}

impl ReadErrorKind {
    pub(crate) fn with_context(self, context: impl Into<String>) -> ReadError {
        ReadError {
            kind: self,
            context: context.into(),
        }
    }
}

/// Errors that can occur when reading declarations of functions manipulating [`Resource`]s
/// from a WASM module.
///
/// [`Resource`]: crate::Resource
#[derive(Debug)]
pub struct ReadError {
    kind: ReadErrorKind,
    context: String,
}

impl fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "failed reading {}: {}", self.context, self.kind)
    }
}

impl error::Error for ReadError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            ReadErrorKind::Utf8(err) => Some(err),
            ReadErrorKind::UnexpectedEof => None,
        }
    }
}
