//! How a session's process is described to `CreateProcessW` (D16, AM51,
//! AM53): its command line, its environment block, and where `PATHEXT`
//! finds a command.
//!
//! Pure functions over strings, compiled on every platform so the Linux
//! suite tests them. Windows passes a process one string, not a list of
//! arguments, and every program splits it again by the C runtime's rules;
//! `cmd.exe` has rules of its own, which is why a batch file's arguments
//! are refused rather than escaped when they hold anything it interprets.

/// Appends `arg` to `line` so the C runtime's parser gives it back exactly:
/// quoted when it is empty or holds whitespace or a quote, with the
/// backslashes before a quote (or before the closing quote) doubled.
pub fn push_arg(line: &mut String, arg: &str, force_quotes: bool) {
    if !line.is_empty() {
        line.push(' ');
    }
    let quote = force_quotes || arg.is_empty() || arg.contains([' ', '\t', '\n', '\x0b', '"']);
    if !quote {
        line.push_str(arg);
        return;
    }
    line.push('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                line.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                line.push('"');
                backslashes = 0;
            }
            _ => {
                line.extend(std::iter::repeat_n('\\', backslashes));
                line.push(c);
                backslashes = 0;
            }
        }
    }
    line.extend(std::iter::repeat_n('\\', backslashes * 2));
    line.push('"');
}

/// The command line for an executable: the program, always quoted, then its
/// arguments.
pub fn command_line(program: &str, args: &[String]) -> String {
    let mut line = String::new();
    push_arg(&mut line, program, true);
    for arg in args {
        push_arg(&mut line, arg, false);
    }
    line
}

/// Whether `program` is run by `cmd.exe` rather than by itself.
pub fn is_batch(program: &str) -> bool {
    let lower = program.to_ascii_lowercase();
    lower.ends_with(".cmd") || lower.ends_with(".bat")
}

/// What `cmd.exe` interprets even inside quotes, or cannot carry in one:
/// expansion (`%`, `!`), quoting itself, and line breaks. Refused, never
/// escaped: the escaping rules for a batch file's arguments are the BatBadBut
/// vulnerability class.
const BATCH_REFUSED: &[char] = &['"', '%', '!', '^', '&', '|', '<', '>', '\r', '\n', '\0'];

/// The command line that runs the batch file `program` through `cmd.exe`
/// (AM53): `/d` skips AutoRun, `/e:ON /v:OFF` fixes the extensions and
/// delayed expansion whatever the registry says, and every argument is
/// quoted. Refuses a program or an argument holding [`BATCH_REFUSED`].
pub fn batch_command_line(cmd_exe: &str, program: &str, args: &[String]) -> Result<String, String> {
    for value in std::iter::once(program).chain(args.iter().map(String::as_str)) {
        if let Some(c) = value.chars().find(|c| BATCH_REFUSED.contains(c)) {
            return Err(format!(
                "{program} is a batch file, and cmd.exe would interpret the {c:?} in {value:?}; \
                 refused rather than escaped"
            ));
        }
    }
    let mut inner = String::new();
    push_arg(&mut inner, program, true);
    for arg in args {
        push_arg(&mut inner, arg, true);
    }
    // With more than two quotes after `/c`, cmd strips the first and the
    // last, leaving `inner` as written.
    let mut line = String::new();
    push_arg(&mut line, cmd_exe, true);
    line.push_str(" /d /e:ON /v:OFF /c \"");
    line.push_str(&inner);
    line.push('"');
    Ok(line)
}

/// The process's environment: `base` with `overrides` applied in order.
/// Names compare without case, as Windows does, and the block is sorted
/// the same way, as `CreateProcessW` expects.
pub fn merge_environment(
    base: impl IntoIterator<Item = (String, String)>,
    overrides: impl IntoIterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = Vec::new();
    for (k, v) in base.into_iter().chain(overrides) {
        if k.is_empty() {
            continue;
        }
        match env.iter_mut().find(|(e, _)| e.eq_ignore_ascii_case(&k)) {
            Some(slot) => slot.1 = v,
            None => env.push((k, v)),
        }
    }
    env.sort_by_key(|(k, _)| k.to_uppercase());
    env
}

/// The file names `command` may be, in the order `cmd` tries them: as given
/// when it already has an extension, otherwise with each of `PATHEXT`'s.
pub fn candidates(command: &str, pathext: &str) -> Vec<String> {
    let name = command.rsplit(['\\', '/']).next().unwrap_or(command);
    if name.contains('.') {
        return vec![command.to_string()];
    }
    let exts: Vec<&str> = pathext
        .split(';')
        .map(str::trim)
        .filter(|e| e.starts_with('.'))
        .collect();
    let exts = if exts.is_empty() {
        vec![".COM", ".EXE", ".BAT", ".CMD"]
    } else {
        exts
    };
    exts.iter()
        .map(|e| format!("{command}{}", e.to_ascii_lowercase()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the C runtime's `CommandLineToArgvW` gives back from a line: the
    /// rules `push_arg` targets, reimplemented so the round trip is tested
    /// here rather than trusted.
    fn split(line: &str) -> Vec<String> {
        let mut args = Vec::new();
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }
            let mut arg = String::new();
            let mut quoted = false;
            while i < chars.len() && (quoted || (chars[i] != ' ' && chars[i] != '\t')) {
                if chars[i] == '\\' {
                    let mut n = 0;
                    while i < chars.len() && chars[i] == '\\' {
                        n += 1;
                        i += 1;
                    }
                    if i < chars.len() && chars[i] == '"' {
                        arg.extend(std::iter::repeat_n('\\', n / 2));
                        if n % 2 == 1 {
                            arg.push('"');
                            i += 1;
                        }
                    } else {
                        arg.extend(std::iter::repeat_n('\\', n));
                    }
                } else if chars[i] == '"' {
                    quoted = !quoted;
                    i += 1;
                } else {
                    arg.push(chars[i]);
                    i += 1;
                }
            }
            args.push(arg);
        }
        args
    }

    #[test]
    fn every_argument_survives_the_c_runtimes_parser() {
        let args: Vec<String> = [
            "plain",
            "",
            "two words",
            r#"a "quoted" word"#,
            r"C:\path\with\trailing\",
            r"C:\Program Files\x\",
            r#"back\\"slash"#,
            "tab\there",
            "--mcp-config=C:\\ProgramData\\StormRuntime\\state\\sessions\\ags_1\\mcp.json",
        ]
        .map(String::from)
        .to_vec();
        let line = command_line(r"C:\Program Files\nodejs\node.exe", &args);
        let parsed = split(&line);
        assert_eq!(parsed[0], r"C:\Program Files\nodejs\node.exe");
        assert_eq!(&parsed[1..], &args[..]);
    }

    #[test]
    fn a_batch_file_runs_through_cmd_with_every_argument_quoted() {
        let line = batch_command_line(
            r"C:\Windows\System32\cmd.exe",
            r"C:\Program Files\nodejs\claude.cmd",
            &[
                "--permission-mode".into(),
                "default".into(),
                r"C:\a b\mcp.json".into(),
            ],
        )
        .unwrap();
        assert_eq!(
            line,
            r#""C:\Windows\System32\cmd.exe" /d /e:ON /v:OFF /c ""C:\Program Files\nodejs\claude.cmd" "--permission-mode" "default" "C:\a b\mcp.json"""#
        );
    }

    #[test]
    fn a_batch_file_refuses_what_cmd_would_interpret() {
        for bad in [
            "%PATH%",
            "a&calc",
            "x|y",
            "\"q",
            "!v!",
            "a^b",
            "<in",
            ">out",
            "two\nlines",
        ] {
            assert!(
                batch_command_line("cmd.exe", "tool.cmd", &[bad.into()]).is_err(),
                "{bad:?} must be refused"
            );
        }
        assert!(batch_command_line("cmd.exe", r"C:\100%\tool.bat", &[]).is_err());
        assert!(is_batch(r"C:\x\CLAUDE.CMD"));
        assert!(is_batch("run.bat"));
        assert!(!is_batch("claude.exe"));
    }

    #[test]
    fn the_environment_merges_without_case_and_is_sorted() {
        let env = merge_environment(
            [
                ("Path".into(), "C:\\old".into()),
                ("b".into(), "1".into()),
                ("A".into(), "2".into()),
            ],
            [
                ("PATH".into(), "C:\\new".into()),
                ("TERM".into(), "xterm-256color".into()),
            ],
        );
        assert_eq!(
            env,
            vec![
                ("A".into(), "2".into()),
                ("b".into(), "1".into()),
                ("Path".into(), "C:\\new".into()),
                ("TERM".into(), "xterm-256color".into()),
            ]
        );
    }

    #[test]
    fn pathext_decides_what_a_bare_command_may_be() {
        assert_eq!(
            candidates("claude", ".COM;.EXE;.BAT;.CMD"),
            vec!["claude.com", "claude.exe", "claude.bat", "claude.cmd"]
        );
        assert_eq!(candidates("claude.cmd", ".EXE"), vec!["claude.cmd"]);
        assert_eq!(candidates(r"C:\x.y\claude", ""), {
            ["com", "exe", "bat", "cmd"]
                .map(|e| format!(r"C:\x.y\claude.{e}"))
                .to_vec()
        });
    }
}
