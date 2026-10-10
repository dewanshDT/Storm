//! The agent's own name for a session, read from its terminal title (OSC 0/2;
//! D15 AM43). The output stream is only observed, never rewritten.

use serde::Serialize;

const NAME_MAX: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Activity {
    Working,
    Idle,
}

/// What a title says, once cleaned.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Title {
    pub name: Option<String>,
    pub activity: Option<Activity>,
}

/// Feeds a session's output through a VT parser and keeps what its latest
/// title says. A name, once given, survives a later title that has none.
#[derive(Default)]
pub struct TitleObserver {
    parser: vte::Parser,
    sink: Sink,
    current: Title,
}

#[derive(Default)]
struct Sink {
    latest: Option<String>,
}

impl vte::Perform for Sink {
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if let [kind, text @ ..] = params
            && matches!(*kind, b"0" | b"2")
        {
            self.latest = Some(String::from_utf8_lossy(&text.join(&b';')).into_owned());
        }
    }
}

impl TitleObserver {
    /// Returns the new state when this output changed it.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<Title> {
        self.parser.advance(&mut self.sink, bytes);
        let raw = self.sink.latest.take()?;
        let mut next = clean(&raw);
        if next.name.is_none() {
            next.name = self.current.name.clone();
        }
        (next != self.current).then(|| {
            self.current = next.clone();
            next
        })
    }

    pub fn current(&self) -> &Title {
        &self.current
    }
}

/// One title as an agent sets it → the name and activity it carries.
pub fn clean(raw: &str) -> Title {
    let mut rest = raw.trim_start();
    let mut activity = None;
    if let Some(first) = rest.chars().next()
        && let Some(a) = glyph(first)
    {
        activity = Some(a);
        rest = &rest[first.len_utf8()..];
    }
    let text: String = rest
        .chars()
        .filter(|c| !c.is_control() || c.is_whitespace())
        .collect();
    let mut name: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some((cut, _)) = name.char_indices().nth(NAME_MAX) {
        name.truncate(cut);
    }
    let named = !name.is_empty()
        && !["claude code", "opencode"].contains(&name.to_lowercase().as_str())
        && !shell_default(&name);
    Title {
        name: named.then_some(name),
        activity,
    }
}

fn glyph(c: char) -> Option<Activity> {
    match c {
        '✳' => Some(Activity::Idle),
        '◐' | '◑' | '◒' | '◓' | '\u{2801}'..='\u{28FF}' => Some(Activity::Working),
        _ => None,
    }
}

/// `user@host:path`, as a shell's prompt command sets it.
fn shell_default(name: &str) -> bool {
    let Some((who, _)) = name.split_once(':') else {
        return false;
    };
    match who.split_once('@') {
        Some((user, host)) => {
            let word = |s: &str| {
                !s.is_empty()
                    && s.chars()
                        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
            };
            word(user) && word(host)
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VECTORS: &str = include_str!("../../../docs/terminal-title-vectors.json");

    fn activity(v: &serde_json::Value) -> Option<Activity> {
        match v.as_str() {
            Some("working") => Some(Activity::Working),
            Some("idle") => Some(Activity::Idle),
            None => None,
            Some(other) => panic!("unknown activity {other}"),
        }
    }

    #[test]
    fn every_vector_cleans_as_recorded() {
        let doc: serde_json::Value = serde_json::from_str(VECTORS).unwrap();
        for v in doc["titles"].as_array().unwrap() {
            let raw = v["raw"].as_str().unwrap();
            let want = Title {
                name: v["name"].as_str().map(str::to_owned),
                activity: activity(&v["activity"]),
            };
            assert_eq!(clean(raw), want, "{raw:?}");
        }
        let long = &doc["long"];
        let raw = "x".repeat(long["raw_length"].as_u64().unwrap() as usize);
        let name = clean(&raw).name.unwrap();
        assert_eq!(
            name.chars().count() as u64,
            long["name_length"].as_u64().unwrap()
        );
    }

    fn osc(title: &str) -> Vec<u8> {
        format!("\x1b]0;{title}\x07").into_bytes()
    }

    #[test]
    fn a_title_split_across_reads_is_seen_once() {
        let bytes = [b"hello ".as_slice(), &osc("✳ Fix the login"), b" world"].concat();
        let mut o = TitleObserver::default();
        let mut seen = Vec::new();
        for byte in &bytes {
            seen.extend(o.feed(std::slice::from_ref(byte)));
        }
        assert_eq!(
            seen,
            [Title {
                name: Some("Fix the login".into()),
                activity: Some(Activity::Idle),
            }]
        );
    }

    #[test]
    fn st_terminated_osc_2_with_semicolons() {
        let mut o = TitleObserver::default();
        let got = o.feed("\x1b]2;◐ a;b\x1b\\".as_bytes()).unwrap();
        assert_eq!(got.name.as_deref(), Some("a;b"));
        assert_eq!(got.activity, Some(Activity::Working));
    }

    #[test]
    fn a_named_session_keeps_its_name_and_only_changes_report() {
        let mut o = TitleObserver::default();
        assert!(o.feed(&osc("✳ Claude Code")).is_some(), "activity is news");
        assert!(o.feed(&osc("✳ Claude Code")).is_none(), "nothing changed");
        let named = o.feed(&osc("◐ Ok")).unwrap();
        assert_eq!(named.name.as_deref(), Some("Ok"));
        // Each spinner frame is the same state.
        assert!(o.feed(&osc("◑ Ok")).is_none());
        let idle = o.feed(&osc("✳ Claude Code")).unwrap();
        assert_eq!(idle.name.as_deref(), Some("Ok"), "never cleared");
        assert_eq!(idle.activity, Some(Activity::Idle));
        assert_eq!(o.current(), &idle);
    }

    #[test]
    fn other_sequences_and_text_say_nothing() {
        let mut o = TitleObserver::default();
        assert!(
            o.feed(b"\x1b[31mred\x1b[0m \x1b]8;;https://x\x07link\x1b]8;;\x07")
                .is_none()
        );
        assert!(o.feed(b"\x1b]11;?\x07").is_none());
        assert_eq!(o.current(), &Title::default());
    }
}
