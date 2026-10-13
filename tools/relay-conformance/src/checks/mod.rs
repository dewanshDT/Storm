//! Every check, tagged with the section of `docs/srp-v1.md` it enforces.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use crate::harness::Ctx;

mod framing;
mod lifecycle;
mod limits;
mod registration;
mod streams;

pub type Run = for<'a> fn(&'a Ctx) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Needs {
    Nothing,
    /// `RELAY_TOFU_URL`.
    Tofu,
    /// `RELAY_MULTI_IP=1`.
    MultiIp,
    /// `RELAY_LATENCY=1`.
    Latency,
}

pub struct Check {
    pub section: &'static str,
    pub name: &'static str,
    pub needs: Needs,
    pub run: Run,
}

macro_rules! check {
    ($section:literal, $f:ident) => {
        check!($section, $f, $crate::checks::Needs::Nothing)
    };
    ($section:literal, $f:ident, $needs:expr) => {
        $crate::checks::Check {
            section: $section,
            name: stringify!($f),
            needs: $needs,
            run: |ctx| Box::pin($f(ctx)),
        }
    };
}
pub(crate) use check;

pub fn all() -> Vec<Check> {
    let mut checks = Vec::new();
    checks.extend(framing::checks());
    checks.extend(registration::checks());
    checks.extend(lifecycle::checks());
    checks.extend(streams::checks());
    checks.extend(limits::checks());
    checks
}
