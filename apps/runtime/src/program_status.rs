//! A program's own report of its state over the OSC 7501 Program Status
//! Protocol (rev 0.4), as decision 155's spike reads it. The host answers the
//! one probe `OSC 7501 ; ?` and keeps the root record; the title glyph stays
//! the fallback for activity whenever no record is live.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::terminal::Activity;

/// The spec rev 0.4 reply, advertising only what Storm maps.
pub const PROBE_REPLY: &[u8] =
    b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\";

/// The longest sequence the spec allows.
pub const MAX_SEQUENCE: usize = 4096;

/// How long after the session's first output a record must arrive once the
/// probe was answered, else activity stays with the title.
pub const FIRST_RECORD_WITHIN: Duration = Duration::from_secs(2);

/// Parse failures, across every session of this host process, after which
/// the host stops answering the probe for new sessions: those keep the title
/// glyph. Reset only by a restart.
pub const HOST_MAX_FAILURES: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub state: State,
    pub kind: Option<String>,
    pub app: Option<String>,
    pub id: Option<String>,
    pub progress: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Probe,
    Record(Record),
    Clear { id: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    TooLong,
    NoState,
    UnknownState,
}

impl ParseError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TooLong => "too_long",
            Self::NoState => "no_state",
            Self::UnknownState => "unknown_state",
        }
    }
}

fn key_ok(k: &str) -> bool {
    !k.is_empty() && k.bytes().all(|b| b.is_ascii_lowercase())
}

fn value_ok(v: &str) -> bool {
    v.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_.,+/=-".contains(&b))
}

/// One OSC 7501 payload, the bytes after `7501;`.
pub fn parse(data: &[u8]) -> Result<Parsed, ParseError> {
    if data.len() + 8 > MAX_SEQUENCE {
        return Err(ParseError::TooLong);
    }
    if data.first() == Some(&b'?') {
        return Ok(Parsed::Probe);
    }
    let text = String::from_utf8_lossy(data);
    let mut pairs: BTreeMap<&str, &str> = BTreeMap::new();
    for pair in text.split(':') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        if key_ok(k) && value_ok(v) {
            pairs.entry(k).or_insert(v);
        }
    }
    let id = pairs
        .get("id")
        .filter(|v| !v.is_empty() && v.split('/').count() <= 8)
        .map(|v| v.to_string());
    let state = match *pairs.get("state").ok_or(ParseError::NoState)? {
        "idle" => State::Idle,
        "working" => State::Working,
        "done" => State::Done,
        "blocked" => State::Blocked,
        "error" => State::Error,
        "clear" => return Ok(Parsed::Clear { id }),
        _ => return Err(ParseError::UnknownState),
    };
    let kind = pairs
        .get("kind")
        .filter(|_| state == State::Blocked)
        .filter(|k| matches!(**k, "permission" | "question" | "auth"))
        .map(|k| k.to_string());
    let app = pairs
        .get("app")
        .filter(|a| !a.is_empty() && a.len() <= 32)
        .map(|a| a.to_string());
    let progress = pairs
        .get("progress")
        .filter(|_| matches!(state, State::Working | State::Blocked))
        .and_then(|p| p.parse::<u8>().ok())
        .filter(|p| *p <= 100);
    Ok(Parsed::Record(Record {
        state,
        kind,
        app,
        id,
        progress,
    }))
}

/// Where a session's activity comes from right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The program reports and its root record is live.
    Program,
    /// The title, as before decision 155: nothing answered or reported, a
    /// `clear`, or no record within [`FIRST_RECORD_WITHIN`] (the program
    /// never took the reply, so its title still has the glyph).
    Title,
    /// The program took the reply, so its title has no glyph, and its last
    /// record did not parse: activity is unknown, never a guessed idle.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// The probe was answered but no record came within
    /// [`FIRST_RECORD_WITHIN`] of the first output.
    NoRecord,
    /// The last record did not parse; until the next one that does.
    Unparsable,
}

impl Fallback {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoRecord => "no_record",
            Self::Unparsable => "unparsable",
        }
    }
}

/// What the host did with one payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// The first probe of the process: the caller queues [`PROBE_REPLY`].
    Answer,
    /// A later probe: never answered twice.
    ProbeAgain,
    Root(Option<Record>),
    Child,
    Discarded(ParseError),
}

/// A session's program status: the root record, its children, and whether
/// the program is still believed.
#[derive(Debug, Default)]
pub struct ProgramStatus {
    answered: Option<Instant>,
    first_output: Option<Instant>,
    root: Option<Record>,
    children: BTreeMap<String, Record>,
    any_record: bool,
    failures: u32,
    unparsed: bool,
    fallback: Option<Fallback>,
}

impl ProgramStatus {
    pub fn output_at(&mut self, now: Instant) {
        self.first_output.get_or_insert(now);
    }

    pub fn see(&mut self, data: &[u8], now: Instant) -> Seen {
        match parse(data) {
            Ok(Parsed::Probe) => {
                if self.answered.is_some() {
                    Seen::ProbeAgain
                } else {
                    self.answered = Some(now);
                    Seen::Answer
                }
            }
            Ok(Parsed::Record(r)) => {
                self.unparsed = false;
                self.any_record = true;
                if self.fallback == Some(Fallback::NoRecord) {
                    self.fallback = None;
                }
                match r.id.clone() {
                    None => {
                        self.root = Some(r.clone());
                        Seen::Root(Some(r))
                    }
                    Some(id) => {
                        if self.children.len() < 32 || self.children.contains_key(&id) {
                            self.children.insert(id, r);
                        }
                        Seen::Child
                    }
                }
            }
            Ok(Parsed::Clear { id: None }) => {
                self.unparsed = false;
                self.any_record = true;
                self.root = None;
                self.children.clear();
                Seen::Root(None)
            }
            Ok(Parsed::Clear { id: Some(id) }) => {
                self.children.remove(&id);
                Seen::Child
            }
            Err(e) => {
                self.failures += 1;
                self.unparsed = true;
                Seen::Discarded(e)
            }
        }
    }

    /// The fallback in force at `now`, if any.
    pub fn fallback(&mut self, now: Instant) -> Option<Fallback> {
        if self.unparsed {
            return Some(Fallback::Unparsable);
        }
        if self.fallback.is_none()
            && self.answered.is_some()
            && !self.any_record
            && self
                .first_output
                .is_some_and(|t| now.duration_since(t) >= FIRST_RECORD_WITHIN)
        {
            self.fallback = Some(Fallback::NoRecord);
        }
        self.fallback
    }

    /// The root record the host believes now.
    pub fn root(&mut self, now: Instant) -> Option<&Record> {
        match self.fallback(now) {
            Some(_) => None,
            None => self.root.as_ref(),
        }
    }

    pub fn children(&self) -> usize {
        self.children.len()
    }

    /// Records of this session that did not parse.
    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// Whether the program was asked.
    pub fn answered(&self) -> bool {
        self.answered.is_some()
    }

    /// The session's activity and where it came from: the program's while
    /// its root record is live, else the title's.
    pub fn activity(
        &mut self,
        title: Option<Activity>,
        now: Instant,
    ) -> (Option<Activity>, Source) {
        if self.fallback(now) == Some(Fallback::Unparsable) {
            return (None, Source::Unknown);
        }
        match self.root(now) {
            Some(r) => (Some(activity_of(r.state)), Source::Program),
            None => (title, Source::Title),
        }
    }

    /// What the program says needs the person, by decision 155's mapping.
    pub fn attention(&mut self, now: Instant) -> Option<&'static str> {
        self.root(now).and_then(attention_of)
    }
}

pub fn activity_of(state: State) -> Activity {
    match state {
        State::Working => Activity::Working,
        _ => Activity::Idle,
    }
}

pub fn attention_of(r: &Record) -> Option<&'static str> {
    (r.state == State::Blocked).then_some(match r.kind.as_deref() {
        Some("permission") => "permission_prompt",
        Some("auth") => "sign_in",
        _ => "question",
    })
}

/// The host's circuit breaker: parse failures across its sessions. Past
/// [`HOST_MAX_FAILURES`] new sessions are not answered, so their program
/// keeps its title glyph and the title gives activity.
#[derive(Debug, Default)]
pub struct Breaker(std::sync::atomic::AtomicU32);

impl Breaker {
    pub const fn new() -> Self {
        Self(std::sync::atomic::AtomicU32::new(0))
    }

    pub fn failed(&self) -> u32 {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
    }

    /// Whether a session starting now gets the probe answered.
    pub fn closed(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst) < HOST_MAX_FAILURES
    }
}

pub static HOST: Breaker = Breaker::new();

/// Spike only: every probe, reply, record, title and derived state, one JSON
/// line each, to the file `STORM_PROGRAM_STATUS_LOG` names.
pub fn spike_log(mut line: serde_json::Value) {
    use std::io::Write;
    static FILE: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> =
        std::sync::OnceLock::new();
    let file = FILE.get_or_init(|| {
        let path = std::env::var_os("STORM_PROGRAM_STATUS_LOG")?;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(std::sync::Mutex::new)
    });
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    line["t"] = serde_json::json!(t);
    tracing::debug!(target: "program_status_raw", "{line}");
    if let Some(f) = file {
        let _ = writeln!(f.lock().unwrap(), "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::TitleObserver;

    /// S0, run 1 and 2 (Claude Code 2.1.323/2.1.325 at 120×40): the bytes
    /// around the probe, and the first record after the reply.
    const S0_STARTUP: &[u8] = b"\x1b7\x1b[r\x1b8\x1b[?25h\x1b[?25l\x1b[?2004h\x1b[?2031h\x1b[?1004h\x1b]0;\xe2\x9c\xb3 Claude Code\x07banner\x1b[>0q\x1b[?u\x1b]7501;?\x07\x1b[c";
    const S0_FIRST_RECORD: &[u8] =
        b"\x1b]7501;state=idle:app=claude-code\x07\x1b]0;Claude Code\x07";

    fn feed(o: &mut TitleObserver, p: &mut ProgramStatus, bytes: &[u8], now: Instant) -> Vec<Seen> {
        o.feed(bytes);
        p.output_at(now);
        o.take_program().iter().map(|d| p.see(d, now)).collect()
    }

    #[test]
    fn the_s0_probe_is_answered_once_and_the_first_record_is_idle() {
        let t0 = Instant::now();
        let mut o = TitleObserver::default();
        let mut p = ProgramStatus::default();
        assert_eq!(feed(&mut o, &mut p, S0_STARTUP, t0), [Seen::Answer]);
        assert_eq!(
            o.current().activity,
            Some(Activity::Idle),
            "the glyph, before the record"
        );
        let seen = feed(
            &mut o,
            &mut p,
            S0_FIRST_RECORD,
            t0 + Duration::from_millis(12),
        );
        let Seen::Root(Some(r)) = &seen[0] else {
            panic!("{seen:?}")
        };
        assert_eq!(
            (r.state, r.app.as_deref()),
            (State::Idle, Some("claude-code"))
        );
        assert_eq!(o.current().activity, None, "the title lost its glyph");
        assert_eq!(
            p.activity(o.current().activity, t0 + Duration::from_secs(5)),
            (Some(Activity::Idle), Source::Program)
        );
        assert_eq!(
            feed(&mut o, &mut p, b"\x1b]7501;?\x1b\\", t0),
            [Seen::ProbeAgain]
        );
    }

    #[test]
    fn a_probe_split_byte_by_byte_is_answered_once() {
        let mut o = TitleObserver::default();
        let mut p = ProgramStatus::default();
        let now = Instant::now();
        let mut seen = Vec::new();
        for b in S0_STARTUP.iter().chain(b"\x1b]7501;?\x07") {
            seen.extend(feed(&mut o, &mut p, std::slice::from_ref(b), now));
        }
        assert_eq!(seen, [Seen::Answer, Seen::ProbeAgain]);
    }

    #[test]
    fn records_parse_by_the_grammar() {
        let r = |s: &str| parse(s.as_bytes());
        assert_eq!(
            r("state=blocked:app=claude-code:kind=permission"),
            Ok(Parsed::Record(Record {
                state: State::Blocked,
                kind: Some("permission".into()),
                app: Some("claude-code".into()),
                id: None,
                progress: None,
            }))
        );
        let Ok(Parsed::Record(k)) = r("state=done:kind=permission:progress=40") else {
            panic!()
        };
        assert_eq!(
            (k.kind, k.progress),
            (None, None),
            "kind and progress only where allowed"
        );
        let Ok(Parsed::Record(m)) = r("state=working:bad pair:X=1:msg=aGk=:progress=101") else {
            panic!()
        };
        assert_eq!((m.state, m.progress), (State::Working, None));
        assert_eq!(r("state=clear"), Ok(Parsed::Clear { id: None }));
        assert_eq!(r("state=bogus"), Err(ParseError::UnknownState));
        assert_eq!(r("app=x"), Err(ParseError::NoState));
        assert_eq!(
            r(&format!("state=idle:msg={}", "A".repeat(4100))),
            Err(ParseError::TooLong)
        );
        let Ok(Parsed::Record(c)) = r("state=working:app=claude-code:id=task1") else {
            panic!()
        };
        assert_eq!(c.id.as_deref(), Some("task1"));
    }

    #[test]
    fn the_mapping_follows_the_decision() {
        let now = Instant::now();
        let mut p = ProgramStatus::default();
        p.output_at(now);
        p.see(b"?", now);
        let mut at = |s: &str| {
            p.see(s.as_bytes(), now);
            (p.activity(None, now).0, p.attention(now))
        };
        assert_eq!(at("state=working"), (Some(Activity::Working), None));
        assert_eq!(
            at("state=blocked:kind=permission"),
            (Some(Activity::Idle), Some("permission_prompt"))
        );
        assert_eq!(
            at("state=blocked:kind=question"),
            (Some(Activity::Idle), Some("question"))
        );
        assert_eq!(
            at("state=blocked"),
            (Some(Activity::Idle), Some("question"))
        );
        assert_eq!(
            at("state=blocked:kind=auth"),
            (Some(Activity::Idle), Some("sign_in"))
        );
        assert_eq!(at("state=done"), (Some(Activity::Idle), None));
        assert_eq!(at("state=error"), (Some(Activity::Idle), None));
        assert_eq!(
            at("state=working:id=a"),
            (Some(Activity::Idle), None),
            "a child is not the root"
        );
        assert_eq!(
            at("state=clear"),
            (None, None),
            "clear falls back to the title"
        );
    }

    #[test]
    fn no_record_within_two_seconds_falls_back_to_the_title() {
        let t0 = Instant::now();
        let mut o = TitleObserver::default();
        let mut p = ProgramStatus::default();
        feed(
            &mut o,
            &mut p,
            b"\x1b]7501;?\x07\x1b]0;\xe2\x97\x90 Busy\x07",
            t0,
        );
        let glyph = o.current().activity;
        assert_eq!(glyph, Some(Activity::Working));
        assert_eq!(p.fallback(t0 + Duration::from_millis(1900)), None);
        assert_eq!(
            p.activity(glyph, t0 + Duration::from_millis(1900)).1,
            Source::Title
        );
        assert_eq!(
            p.fallback(t0 + FIRST_RECORD_WITHIN),
            Some(Fallback::NoRecord)
        );
        assert_eq!(
            p.activity(glyph, t0 + Duration::from_secs(3)),
            (Some(Activity::Working), Source::Title)
        );
        p.see(b"state=done", t0 + Duration::from_secs(4));
        assert_eq!(
            p.fallback(t0 + Duration::from_secs(4)),
            None,
            "a late record is believed"
        );
    }

    #[test]
    fn a_record_that_does_not_parse_makes_activity_unknown_not_idle() {
        let now = Instant::now();
        let mut o = TitleObserver::default();
        let mut p = ProgramStatus::default();
        feed(&mut o, &mut p, S0_STARTUP, now);
        feed(&mut o, &mut p, S0_FIRST_RECORD, now);
        feed(
            &mut o,
            &mut p,
            b"\x1b]7501;state=working:app=claude-code\x07",
            now,
        );
        let seen = feed(&mut o, &mut p, b"\x1b]7501;state=thinking\x07", now);
        assert_eq!(seen, [Seen::Discarded(ParseError::UnknownState)]);
        assert_eq!(o.current().activity, None, "no glyph to fall back to");
        assert_eq!(
            p.activity(o.current().activity, now),
            (None, Source::Unknown)
        );
        assert_eq!(p.attention(now), None);
        assert_eq!(p.failures(), 1);
        feed(&mut o, &mut p, b"\x1b]7501;app=claude-code\x07", now);
        assert_eq!(p.activity(None, now), (None, Source::Unknown));
        assert_eq!(p.failures(), 2);
        feed(
            &mut o,
            &mut p,
            b"\x1b]7501;state=done:app=claude-code\x07",
            now,
        );
        assert_eq!(
            p.activity(None, now),
            (Some(Activity::Idle), Source::Program),
            "a good record is believed again"
        );
    }

    #[test]
    fn the_breaker_opens_after_three_failures_across_sessions() {
        let b = Breaker::new();
        assert!(b.closed());
        b.failed();
        b.failed();
        assert!(b.closed());
        assert_eq!(b.failed(), HOST_MAX_FAILURES);
        assert!(!b.closed(), "new sessions are not answered");
    }

    #[test]
    fn an_unanswered_program_keeps_its_glyph_and_title_activity() {
        // What a session started after the breaker opened sees: the host
        // never answers, so S0 run 1's glyph title stays.
        let now = Instant::now();
        let mut o = TitleObserver::default();
        let mut p = ProgramStatus::default();
        o.feed(S0_STARTUP);
        p.output_at(now);
        o.take_program();
        assert!(!p.answered());
        assert_eq!(
            p.activity(o.current().activity, now + Duration::from_secs(5)),
            (Some(Activity::Idle), Source::Title)
        );
        o.feed("\x1b]0;\u{2810} Claude Code\x07".as_bytes());
        assert_eq!(
            p.activity(o.current().activity, now + Duration::from_secs(6)),
            (Some(Activity::Working), Source::Title)
        );
    }

    #[test]
    fn every_captured_record_maps_as_the_vectors_say() {
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("../../../docs/program-status-vectors.json"))
                .unwrap();
        for v in doc["records"].as_array().unwrap() {
            let now = Instant::now();
            let mut p = ProgramStatus::default();
            p.output_at(now);
            assert_eq!(p.see(b"?", now), Seen::Answer);
            if let Some(before) = v["after"].as_str() {
                p.see(before.as_bytes(), now);
            }
            p.see(v["raw"].as_str().unwrap().as_bytes(), now);
            let (activity, source) = p.activity(None, now);
            let want = match v["activity"].as_str() {
                Some("working") => Some(Activity::Working),
                Some("idle") => Some(Activity::Idle),
                _ => None,
            };
            assert_eq!(activity, want, "{}", v["case"]);
            assert_eq!(p.attention(now), v["attention"].as_str(), "{}", v["case"]);
            let source = match source {
                Source::Program => "program",
                Source::Title => "title",
                Source::Unknown => "unknown",
            };
            assert_eq!(source, v["source"], "{}", v["case"]);
        }
        assert_eq!(doc["reply"].as_str().unwrap().as_bytes(), PROBE_REPLY);
    }

    #[test]
    fn a_program_that_never_probes_keeps_the_title() {
        let now = Instant::now();
        let mut p = ProgramStatus::default();
        p.output_at(now);
        assert_eq!(p.fallback(now + Duration::from_secs(10)), None);
        assert_eq!(
            p.activity(Some(Activity::Working), now + Duration::from_secs(10)),
            (Some(Activity::Working), Source::Title)
        );
    }
}
