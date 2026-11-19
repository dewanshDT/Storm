//! Shared diagnostics for decision 162: one redaction function, a
//! `tracing` log ring buffer built on it, and the `Diag` bundle shape —
//! used by both `storm-server` and `storm-runtime`, and by nothing else
//! (freeze §5.8, the R6 reasoning): a leaf crate, not a workspace joining
//! the two. It depends on neither, and holds no wire format of theirs —
//! only what 162c says every diagnostics bundle must scrub and carry.

pub mod diag;
pub mod redact;
pub mod ring;

pub use diag::{Diag, RuntimeDiag, SCHEMA, ServerDiag};
pub use redact::{Sensitive, SensitiveKind, redact, redact_default};
pub use ring::{Ring, RingWriter};

/// Builds the second `fmt` layer a binary's `main` adds to its existing
/// subscriber: same formatting, a different writer. Call once at startup,
/// alongside (never instead of) the stdout/`--log-file` layer, and keep
/// the returned [`Ring`] to read from later — a CLI `diag` subcommand has
/// no running process to ask, so it reads the log *file* instead and
/// never calls this.
pub fn ring_layer<S>(ring: Ring) -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(RingWriter::new(ring))
}
