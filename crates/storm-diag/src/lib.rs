//! Shared diagnostics for decision 162: redaction, a log ring buffer
//! built on it, and the `Diag` bundle shape. A path dependency of both
//! `storm-server` and `storm-runtime`, not a workspace joining them.

pub mod diag;
pub mod redact;
pub mod ring;

pub use diag::{Diag, RuntimeDiag, SCHEMA, ServerDiag};
pub use redact::{Sensitive, SensitiveKind, redact, redact_default};
pub use ring::{Ring, RingWriter};

/// A second `fmt` layer over the ring, added alongside (never instead
/// of) a binary's existing subscriber layer.
pub fn ring_layer<S>(ring: Ring) -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(RingWriter::new(ring))
}
