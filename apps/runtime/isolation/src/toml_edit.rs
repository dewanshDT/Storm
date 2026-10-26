//! Sets `isolation` in the operator's `runtime.toml` by rewriting one line.
//! The file is the operator's, with their comments, so it is never parsed
//! and re-serialized; the result is parsed only to check it.

use anyhow::{Result, bail};

use crate::config::{HelperConfig, Isolation};

fn value(mode: Isolation) -> &'static str {
    match mode {
        Isolation::Off => "off",
        Isolation::PerSession => "per-session",
    }
}

fn is_isolation_key(line: &str) -> bool {
    line.trim_start()
        .strip_prefix("isolation")
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

pub fn set_isolation(text: &str, mode: Isolation) -> Result<String> {
    let line = format!("isolation = \"{}\"", value(mode));
    let lines: Vec<&str> = text.lines().collect();
    // Top-level keys come before the first table.
    let tables = lines
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .unwrap_or(lines.len());
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    match lines[..tables].iter().position(|l| is_isolation_key(l)) {
        Some(i) => out[i] = line,
        None => {
            let mut block = vec![
                "# Per-session isolation (decision 136). Set by `storm-runtime-isolation on`"
                    .to_string(),
                "# and `off`, which also convert the workspaces; never edit it by hand."
                    .to_string(),
                line,
            ];
            if tables < lines.len() {
                block.push(String::new());
            } else if out.last().is_some_and(|l| !l.trim().is_empty()) {
                block.insert(0, String::new());
            }
            out.splice(tables..tables, block);
        }
    }
    let mut result = out.join("\n");
    result.push('\n');
    match HelperConfig::parse(&result) {
        Ok(c) if c.isolation == mode => {}
        Ok(_) => bail!("runtime.toml sets isolation somewhere this command cannot change it"),
        Err(e) => bail!("runtime.toml would not parse after the change: {e}"),
    }
    if toml::from_str::<toml::Table>(&result).is_err() {
        bail!("runtime.toml would not parse after the change");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_existing_key_is_replaced_in_place() {
        let text =
            "# mine\nworkspace_roots = [\"/w\"]\nisolation = \"off\" # was off\nmax_sessions = 3\n";
        let out = set_isolation(text, Isolation::PerSession).unwrap();
        assert_eq!(
            out,
            "# mine\nworkspace_roots = [\"/w\"]\nisolation = \"per-session\"\nmax_sessions = 3\n"
        );
        assert_eq!(
            set_isolation(&out, Isolation::Off).unwrap().lines().nth(2),
            Some("isolation = \"off\"")
        );
    }

    #[test]
    fn a_missing_key_goes_before_the_first_table() {
        let text = "workspace_roots = [\"/w\"]\n# isolation = \"off\"\n\n[[providers]]\nid = \"shell\"\nisolation_note = 1\n";
        let out = set_isolation(text, Isolation::PerSession).unwrap();
        let tables = out.find("[[providers]]").unwrap();
        let key = out.find("isolation = \"per-session\"").unwrap();
        assert!(key < tables, "{out}");
        assert!(out.contains("# isolation = \"off\""), "comments are kept");
        assert!(out.ends_with("id = \"shell\"\nisolation_note = 1\n"));
    }

    #[test]
    fn an_empty_or_missing_file_gets_the_key() {
        let out = set_isolation("", Isolation::PerSession).unwrap();
        assert_eq!(
            HelperConfig::parse(&out).unwrap().isolation,
            Isolation::PerSession
        );
        let out = set_isolation("max_sessions = 2", Isolation::PerSession).unwrap();
        assert!(
            out.starts_with("max_sessions = 2\n\n# Per-session"),
            "{out}"
        );
    }

    #[test]
    fn a_file_that_would_not_parse_is_refused() {
        assert!(set_isolation("workspace_roots = [", Isolation::PerSession).is_err());
        // A key inside a table is not the top-level one; adding another
        // would leave the file meaning something else.
        assert!(set_isolation("[x]\nisolation = \"off\"\n", Isolation::PerSession).is_ok());
    }
}
