//! The agent's own name for a session, read from its terminal title (OSC 0/2;
//! D15 AM43), and its desktop notifications (OSC 9/777/99; decision 161,
//! AM126). The output stream is only observed, never rewritten.

use std::collections::VecDeque;

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
/// The same pass collects OSC 7501 payloads (decision 155) for the caller.
pub struct TitleObserver {
    parser: vte::Parser,
    sink: Sink,
    current: Title,
}

impl Default for TitleObserver {
    fn default() -> Self {
        Self::new(true, false)
    }
}

struct Sink {
    titles: bool,
    program_status: bool,
    latest: Option<String>,
    program: Vec<Vec<u8>>,
}

impl vte::Perform for Sink {
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        match params {
            [kind, text @ ..] if self.titles && matches!(*kind, b"0" | b"2") => {
                self.latest = Some(String::from_utf8_lossy(&text.join(&b';')).into_owned());
            }
            [b"7501", data @ ..] if self.program_status => self.program.push(data.join(&b';')),
            _ => {}
        }
    }
}

impl TitleObserver {
    /// `titles: false` reads no titles; `program_status` keeps OSC 7501
    /// payloads for [`Self::take_program`].
    pub fn new(titles: bool, program_status: bool) -> Self {
        Self {
            parser: vte::Parser::new(),
            sink: Sink {
                titles,
                program_status,
                latest: None,
                program: Vec::new(),
            },
            current: Title::default(),
        }
    }

    /// The OSC 7501 payloads seen since the last call, in order.
    pub fn take_program(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.sink.program)
    }

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

/// A terminal desktop notification, forwarded but never answered (decision
/// 161, AM126).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub body: String,
    pub source_form: &'static str,
}

const NOTICE_TITLE_MAX: usize = 200;
const NOTICE_BODY_MAX: usize = 1000;
const NOTICE_ID_MAX: usize = 64;
/// At most this many OSC 99 notifications assembled at once; a new `i=` past
/// the bound is ignored rather than tracked without end.
const KITTY_PENDING_MAX: usize = 16;

fn bounded(mut s: String, max_chars: usize) -> String {
    if let Some((cut, _)) = s.char_indices().nth(max_chars) {
        s.truncate(cut);
    }
    s
}

fn joined_text(parts: &[&[u8]]) -> String {
    String::from_utf8_lossy(&parts.join(&b';')).into_owned()
}

fn is_ascii_digits(b: &[u8]) -> bool {
    !b.is_empty() && b.iter().all(u8::is_ascii_digit)
}

#[derive(Default)]
struct KittyPending {
    title: Option<String>,
    body: Option<String>,
}

/// Kitty's own defaults (its desktop notifications protocol): `d` absent
/// means done, `p` absent means `title`, `i` is optional.
struct KittyMeta {
    id: Option<String>,
    field: String,
    done: bool,
    /// `e=1`/`e=2`: the payload is encoded (base64). Never decoded here —
    /// the text is dropped rather than shown garbled.
    encoded: bool,
}

impl Default for KittyMeta {
    fn default() -> Self {
        Self {
            id: None,
            field: "title".to_string(),
            done: true,
            encoded: false,
        }
    }
}

/// `i=`, `p=`, `d=` and `e=` out of an OSC 99 message's colon-separated
/// metadata.
fn kitty_meta(meta: &[u8]) -> KittyMeta {
    let text = String::from_utf8_lossy(meta);
    let mut m = KittyMeta::default();
    for pair in text.split(':') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        match k {
            "i" => m.id = Some(bounded(v.to_string(), NOTICE_ID_MAX)),
            "p" => m.field = v.to_string(),
            "d" => m.done = v != "0",
            "e" => m.encoded = v != "0",
            _ => {}
        }
    }
    m
}

fn kitty_set(entry: &mut KittyPending, field: &str, text: String) {
    match field {
        "title" => entry.title = Some(bounded(text, NOTICE_TITLE_MAX)),
        "body" => entry.body = Some(bounded(text, NOTICE_BODY_MAX)),
        _ => {}
    }
}

#[derive(Default)]
struct NoticeSink {
    /// Pending OSC 99 (kitty) notifications by `i=`, oldest first, so a
    /// full table evicts the oldest rather than refusing a new id.
    kitty: VecDeque<(String, KittyPending)>,
    notices: Vec<Notice>,
}

impl vte::Perform for NoticeSink {
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        match params {
            [b"9", rest @ ..] => self.osc9(rest),
            [b"777", b"notify", title, rest @ ..] => self.osc777(title, rest),
            [b"99", meta, payload @ ..] => self.osc99(meta, payload),
            _ => {}
        }
    }
}

impl NoticeSink {
    /// iTerm2's form, `OSC 9 ; <text>`. ConEmu's and iTerm2's own progress
    /// forms, `OSC 9 ; <digit> ; …` (for example `9;4`), are not
    /// notifications and are ignored (AM126) — so is a body that happens to
    /// be only digits (`OSC 9;42`), the same test a lone progress field
    /// would fail; accepted, since real notification text is rarely bare
    /// digits.
    fn osc9(&mut self, rest: &[&[u8]]) {
        if rest.first().is_some_and(|p| is_ascii_digits(p)) {
            return;
        }
        let body = bounded(joined_text(rest), NOTICE_BODY_MAX);
        if !body.is_empty() {
            self.notices.push(Notice {
                title: None,
                body,
                source_form: "osc9",
            });
        }
    }

    /// Ghostty's and Codex's form, `OSC 777 ; notify ; <title> ; <message>`.
    fn osc777(&mut self, title: &[u8], rest: &[&[u8]]) {
        let title = bounded(
            String::from_utf8_lossy(title).into_owned(),
            NOTICE_TITLE_MAX,
        );
        let body = bounded(joined_text(rest), NOTICE_BODY_MAX);
        if !title.is_empty() || !body.is_empty() {
            self.notices.push(Notice {
                title: (!title.is_empty()).then_some(title),
                body,
                source_form: "osc777",
            });
        }
    }

    /// Kitty's form, assembled across its `i=` messages until done (`d`
    /// absent, or present and not `0`). A query (`p=?`, OpenCode's
    /// capability probe) is never answered and never becomes a notice
    /// (AM22, AM126). A message with no `i=` is a one-shot: there is
    /// nothing to assemble it with later, so it reports only if it is
    /// itself done.
    fn osc99(&mut self, meta: &[u8], payload: &[&[u8]]) {
        let meta = kitty_meta(meta);
        if meta.field == "?" {
            return;
        }
        let text = (!meta.encoded && !payload.is_empty()).then(|| joined_text(payload));
        let Some(id) = meta.id else {
            let mut entry = KittyPending::default();
            if let Some(text) = text {
                kitty_set(&mut entry, &meta.field, text);
            }
            if meta.done {
                self.finish(entry);
            }
            return;
        };
        if !self.kitty.iter().any(|(i, _)| *i == id) {
            if self.kitty.len() >= KITTY_PENDING_MAX {
                self.kitty.pop_front();
            }
            self.kitty.push_back((id.clone(), KittyPending::default()));
        }
        if let Some(text) = text
            && let Some((_, entry)) = self.kitty.iter_mut().find(|(i, _)| *i == id)
        {
            kitty_set(entry, &meta.field, text);
        }
        if meta.done
            && let Some(pos) = self.kitty.iter().position(|(i, _)| *i == id)
        {
            let (_, entry) = self.kitty.remove(pos).unwrap();
            self.finish(entry);
        }
    }

    fn finish(&mut self, entry: KittyPending) {
        if entry.title.is_some() || entry.body.is_some() {
            self.notices.push(Notice {
                title: entry.title,
                body: entry.body.unwrap_or_default(),
                source_form: "osc99",
            });
        }
    }
}

/// Reads OSC 9, 777 and 99 (desktop notifications) out of a session's
/// output, alongside [`TitleObserver`]'s own pass, and never answers any of
/// them — OpenCode's OSC 99 `p=?` probe included (decision 161, AM126).
pub struct NoticeObserver {
    parser: vte::Parser,
    sink: NoticeSink,
}

impl Default for NoticeObserver {
    fn default() -> Self {
        Self {
            parser: vte::Parser::new(),
            sink: NoticeSink::default(),
        }
    }
}

impl NoticeObserver {
    /// The notices seen since the last call, in order.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Notice> {
        self.parser.advance(&mut self.sink, bytes);
        std::mem::take(&mut self.sink.notices)
    }
}

/// Reads a program's `CSI ? 2031 h`/`l` (DEC 2031, decision 157's
/// colour-scheme report mode) out of a session's output, alongside
/// [`NoticeObserver`]'s own pass (decision 157, theme catch-up). This is the
/// host's half of the client's catch-up: a client attaching fresh learns
/// whether the program wants a report even if the mode-set sequence already
/// scrolled out of its own replay.
pub struct SchemeReportObserver {
    parser: vte::Parser,
    sink: SchemeReportSink,
}

impl Default for SchemeReportObserver {
    fn default() -> Self {
        Self {
            parser: vte::Parser::new(),
            sink: SchemeReportSink::default(),
        }
    }
}

/// `None` until the program's first `h`/`l`: unlike [`current`]'s caller,
/// which must settle on a default, this tells "never asked" apart from
/// "asked, and said off".
#[derive(Default)]
struct SchemeReportSink {
    enabled: Option<bool>,
}

impl vte::Perform for SchemeReportSink {
    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        if intermediates != b"?" || !matches!(action, 'h' | 'l') {
            return;
        }
        if params.iter().any(|p| p == [2031]) {
            self.enabled = Some(action == 'h');
        }
    }
}

impl SchemeReportObserver {
    /// Returns the new state when this output changed it.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<bool> {
        let before = self.sink.enabled;
        self.parser.advance(&mut self.sink, bytes);
        (self.sink.enabled != before)
            .then_some(self.sink.enabled)
            .flatten()
    }

    /// `None` before the program has ever set or reset DEC 2031.
    pub fn current(&self) -> Option<bool> {
        self.sink.enabled
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

    fn notice(body: &str) -> Notice {
        Notice {
            title: None,
            body: body.into(),
            source_form: "osc9",
        }
    }

    /// Claude Code 2.1.325's iTerm2 channel: `OSC 9 ; <title>: <message>`.
    #[test]
    fn claude_code_iterm2_osc9_is_a_notice() {
        let mut o = NoticeObserver::default();
        let got = o.feed(b"\x1b]9;tidier: Approve Bash?\x07");
        assert_eq!(got, [notice("tidier: Approve Bash?")]);
    }

    /// Claude Code 2.1.325's kitty channel: three OSC 99 messages sharing
    /// one `i=`. It completes at the second (`p=body`, no `d=` → done by
    /// kitty's own default); the trailing `d=1:a=focus` carries no text and
    /// the id is already gone, so it reports nothing.
    #[test]
    fn claude_code_kitty_osc99_is_assembled_into_one_notice() {
        let mut o = NoticeObserver::default();
        assert!(o.feed(b"\x1b]99;i=42:d=0:p=title;tidier\x07").is_empty());
        let got = o.feed(b"\x1b]99;i=42:p=body;Approve Bash?\x07");
        assert_eq!(
            got,
            [Notice {
                title: Some("tidier".into()),
                body: "Approve Bash?".into(),
                source_form: "osc99",
            }]
        );
        assert!(o.feed(b"\x1b]99;i=42:d=1:a=focus\x07").is_empty());
    }

    /// Kitty's own defaults: `i` is optional, `p` defaults to `title`, and
    /// `d` absent means done — the simplest possible form.
    #[test]
    fn a_bare_osc_99_with_no_metadata_reports_one_notice() {
        let mut o = NoticeObserver::default();
        let got = o.feed(b"\x1b]99;;Hello\x07");
        assert_eq!(
            got,
            [Notice {
                title: Some("Hello".into()),
                body: "".into(),
                source_form: "osc99",
            }]
        );
    }

    /// A body with no `d=` field completes immediately (kitty's default).
    #[test]
    fn an_osc_99_body_with_no_d_completes() {
        let mut o = NoticeObserver::default();
        let got = o.feed(b"\x1b]99;i=1:p=body;hello\x07");
        assert_eq!(
            got,
            [Notice {
                title: None,
                body: "hello".into(),
                source_form: "osc99",
            }]
        );
    }

    /// The pending table evicts the oldest open id rather than refusing a
    /// new one, so 17 ids left open don't disable OSC 99.
    #[test]
    fn seventeen_open_ids_evict_the_oldest_rather_than_refuse_the_newest() {
        let mut o = NoticeObserver::default();
        for i in 0..17 {
            assert!(
                o.feed(format!("\x1b]99;i={i}:d=0:p=title;t{i}\x07").as_bytes())
                    .is_empty()
            );
        }
        // Evicted: completing it starts a fresh, empty one, so nothing to
        // report.
        assert!(o.feed(b"\x1b]99;i=0:d=1\x07").is_empty());
        // The newest id survived the eviction and still assembles.
        let got = o.feed(b"\x1b]99;i=16:d=1:p=body;done\x07");
        assert_eq!(
            got,
            [Notice {
                title: Some("t16".into()),
                body: "done".into(),
                source_form: "osc99",
            }]
        );
    }

    /// A base64-encoded payload (`e=1`) is dropped, never shown garbled.
    #[test]
    fn an_encoded_osc_99_payload_is_dropped_not_garbled() {
        let mut o = NoticeObserver::default();
        assert!(
            o.feed(b"\x1b]99;i=9:e=1:p=title;aGVsbG8=\x07").is_empty(),
            "nothing set, so nothing to report once done"
        );
    }

    /// Claude Code 2.1.325's ghostty channel:
    /// `OSC 777 ; notify ; <title> ; <message>`.
    #[test]
    fn claude_code_ghostty_osc777_is_a_notice() {
        let mut o = NoticeObserver::default();
        let got = o.feed(b"\x1b]777;notify;tidier;Approve Bash?\x07");
        assert_eq!(
            got,
            [Notice {
                title: Some("tidier".into()),
                body: "Approve Bash?".into(),
                source_form: "osc777",
            }]
        );
    }

    /// Codex 0.169.0's ghostty channel, with its own fullwidth `；` standing
    /// in for a literal semicolon inside the text — carried through as-is.
    #[test]
    fn codex_osc777_with_fullwidth_semicolon_is_a_notice() {
        let mut o = NoticeObserver::default();
        let got = o.feed("\x1b]777;notify;Codex;turn complete；see terminal\x07".as_bytes());
        assert_eq!(
            got,
            [Notice {
                title: Some("Codex".into()),
                body: "turn complete；see terminal".into(),
                source_form: "osc777",
            }]
        );
    }

    /// ConEmu's (and iTerm2's own) progress form, `OSC 9 ; <digit> ; …`, is
    /// not a notification.
    #[test]
    fn conemu_progress_osc9_4_is_ignored() {
        let mut o = NoticeObserver::default();
        assert!(o.feed(b"\x1b]9;4;0;50\x07").is_empty());
    }

    #[test]
    fn a_notice_split_across_reads_is_seen_once() {
        let bytes = [
            b"before ".as_slice(),
            b"\x1b]777;notify;tidier;Approve Bash?\x07",
            b" after",
        ]
        .concat();
        let mut o = NoticeObserver::default();
        let mut seen = Vec::new();
        for byte in &bytes {
            seen.extend(o.feed(std::slice::from_ref(byte)));
        }
        assert_eq!(
            seen,
            [Notice {
                title: Some("tidier".into()),
                body: "Approve Bash?".into(),
                source_form: "osc777",
            }]
        );
    }

    /// OpenCode 1.19.5's startup probe. Storm answers nothing, and it is
    /// never mistaken for a notice (AM22, AM126).
    #[test]
    fn opencode_osc99_probe_is_not_answered_or_reported() {
        let mut o = NoticeObserver::default();
        let got = o.feed(b"\x1b]99;i=opentui-notifications:p=?\x07");
        assert!(got.is_empty());
    }

    #[test]
    fn a_title_and_body_are_bounded() {
        let mut o = NoticeObserver::default();
        let long_title = "x".repeat(NOTICE_TITLE_MAX + 50);
        let long_body = "y".repeat(NOTICE_BODY_MAX + 50);
        let got = o.feed(format!("\x1b]777;notify;{long_title};{long_body}\x07").as_bytes());
        let [got] = <[Notice; 1]>::try_from(got).unwrap();
        assert_eq!(got.title.unwrap().chars().count(), NOTICE_TITLE_MAX);
        assert_eq!(got.body.chars().count(), NOTICE_BODY_MAX);
    }

    #[test]
    fn a_notice_without_an_id_or_with_only_a_close_message_reports_nothing() {
        let mut o = NoticeObserver::default();
        assert!(o.feed(b"\x1b]99;d=0:p=title;orphan\x07").is_empty());
        assert!(o.feed(b"\x1b]99;i=1:d=1:a=close\x07").is_empty());
    }

    #[test]
    fn scheme_report_mode_is_unknown_until_the_program_sets_or_resets_it() {
        let o = SchemeReportObserver::default();
        assert_eq!(o.current(), None);
    }

    #[test]
    fn scheme_report_mode_set_and_reset() {
        let mut o = SchemeReportObserver::default();
        assert_eq!(o.feed(b"\x1b[?2031h"), Some(true));
        assert_eq!(o.current(), Some(true));
        // Feeding the same state again reports no change.
        assert_eq!(o.feed(b"\x1b[?2031h"), None);
        assert_eq!(o.feed(b"\x1b[?2031l"), Some(false));
        assert_eq!(o.current(), Some(false));
    }

    #[test]
    fn scheme_report_mode_split_across_writes() {
        let seq = b"\x1b[?2031h";
        for cut in 1..seq.len() {
            let (a, b) = seq.split_at(cut);
            let mut o = SchemeReportObserver::default();
            o.feed(a);
            assert_eq!(o.feed(b), Some(true), "cut at {cut}");
        }
    }

    #[test]
    fn an_unrelated_private_mode_or_a_combined_set_is_never_mistaken_for_2031() {
        let mut o = SchemeReportObserver::default();
        assert_eq!(o.feed(b"\x1b[?25h"), None);
        assert_eq!(o.feed(b"\x1b[?1049h"), None);
        // 2031 alongside another mode in one sequence still sets it.
        assert_eq!(o.feed(b"\x1b[?1004;2031h"), Some(true));
        assert_eq!(o.current(), Some(true));
        // A plain (non-private) CSI with 2031 is not a mode report.
        let mut o2 = SchemeReportObserver::default();
        assert_eq!(o2.feed(b"\x1b[2031h"), None);
        assert_eq!(o2.current(), None);
    }
}
