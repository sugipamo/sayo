//! Typed references to Host-owned values. The evaluator never dereferences them.

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalKind {
    /// An immutable value; processing produces a new reference.
    Blob,
    /// A connection to an external target whose state may change.
    Handle,
}

/// A nominal reference, not a capability or proof that a target exists.
/// IDs are visible in source and traces and must not contain credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalReference {
    kind: ExternalKind,
    id: String,
}

impl ExternalReference {
    pub fn new(kind: ExternalKind, id: impl Into<String>) -> Result<Self> {
        let id = id.into();
        if id.is_empty()
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        {
            return Err(Error::Parse(
                "external reference ID must contain only ASCII letters, digits, _, -, or .".into(),
            ));
        }
        Ok(Self { kind, id })
    }

    pub fn kind(&self) -> ExternalKind {
        self.kind
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn parse(source: &str) -> Result<Self> {
        let (kind, id) = source.split_once(':').ok_or_else(|| {
            Error::Parse("external reference must be blob:ID or handle:ID".into())
        })?;
        let kind = match kind {
            "blob" => ExternalKind::Blob,
            "handle" => ExternalKind::Handle,
            _ => return Err(Error::Parse("unknown external reference kind".into())),
        };
        Self::new(kind, id)
    }

    pub fn render_source(&self) -> String {
        let kind = match self.kind {
            ExternalKind::Blob => "blob",
            ExternalKind::Handle => "handle",
        };
        format!("{kind}:{}", self.id)
    }
}
