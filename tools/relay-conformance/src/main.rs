//! `relay-conformance` — a black-box SRP v1 conformance suite.

mod fixtures;

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("gen-fixtures") => {
            let dir = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures"));
            fixtures::write(&dir)
        }
        _ => anyhow::bail!("usage: relay-conformance gen-fixtures [DIR]"),
    }
}
