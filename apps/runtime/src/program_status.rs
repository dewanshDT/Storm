//! A program's own report of its state over the OSC 7501 Program Status
//! Protocol (rev 0.4; decision 155, AM112, AM113). The host answers the one
//! probe `OSC 7501 ; ?` once per process and keeps the root record; the
//! title glyph stays the fallback for activity whenever no record is live.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use data_encoding::BASE64;

use crate::terminal::Activity;

/// The spec rev 0.4 reply, advertising only what Storm maps.
pub const PROBE_REPLY: &[u8] =
    b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\";

/// The longest sequence the spec allows.
pub const MAX_SEQUENCE: usize = 4096;
const MSG_MAX: usize = 2048;
const TITLE_MAX: usize = 192;

/// Once the probe is answered, a record must come this soon after the
/// session's first output, else activity stays with the title.
pub const FIRST_RECORD_WITHIN: Duration = Duration::from_secs(2);

/// Parse failures across every session of this host process after which
/// the host stops answering the probe for new sessions until it restarts.
pub const HOST_MAX_FAILURES: u32 = 3;

pub const SOURCE: &str = "program_status";

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

/// One record. `msg` and `title` are agent output: kept on the host, never
/// posted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub state: State,
    pub kind: Option<String>,
    pub app: Option<String>,
    pub id: Option<String>,
    pub progress: Option<u8>,
    pub msg: Option<Vec<u8>>,
    pub title: Option<Vec<u8>>,
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

/// A payload that did not parse, and whether it named a child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Malformed {
    pub error: ParseError,
    pub child: bool,
}

fn key_ok(k: &str) -> bool {
    !k.is_empty() && k.bytes().all(|b| b.is_ascii_lowercase())
}

fn value_ok(v: &str) -> bool {
    v.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_.,+/=-".contains(&b))
}

fn decoded(v: Option<&&str>, max: usize) -> Option<Vec<u8>> {
    BASE64.decode(v?.as_bytes()).ok().filter(|d| d.len() <= max)
}

/// One OSC 7501 payload, the bytes after `7501;`.
pub fn parse(data: &[u8]) -> Result<Parsed, Malformed> {
    if data.len() + "\x1b]7501;\x1b\\".len() > MAX_SEQUENCE {
        return Err(Malformed {
            error: ParseError::TooLong,
            child: false,
        });
    }
    if data.first() == Some(&b'?') {
        return Ok(Parsed::Probe);
    }
    let text = String::from_utf8_lossy(data);
    let mut pairs: BTreeMap<&str, &str> = BTreeMap::new();
    for pair in text.split(':') {
        if let Some((k, v)) = pair.split_once('=')
            && key_ok(k)
            && value_ok(v)
        {
            pairs.entry(k).or_insert(v);
        }
    }
    let id = pairs
        .get("id")
        .filter(|v| !v.is_empty() && v.split('/').count() <= 8)
        .map(|v| v.to_string());
    let malformed = |error| Malformed {
        error,
        child: id.is_some(),
    };
    let state = match *pairs.get("state").ok_or(malformed(ParseError::NoState))? {
        "idle" => State::Idle,
        "working" => State::Working,
        "done" => State::Done,
        "blocked" => State::Blocked,
        "error" => State::Error,
        "clear" => return Ok(Parsed::Clear { id }),
        _ => return Err(malformed(ParseError::UnknownState)),
    };
    let kind = pairs
        .get("kind")
        .filter(|k| state == State::Blocked && matches!(**k, "permission" | "question" | "auth"))
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
        msg: decoded(pairs.get("msg"), MSG_MAX),
        title: decoded(pairs.get("title"), TITLE_MAX),
    }))
}

/// Where a session's activity comes from right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The root record is live.
    Program,
    /// The title, as before decision 155: nothing answered or reported yet,
    /// a `clear`, or no record within [`FIRST_RECORD_WITHIN`].
    Title,
    /// The program took the reply, so its title has no glyph, and its last
    /// record did not parse: unknown, never a guessed idle.
    Unknown,
}

/// What the host did with one payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// The first probe of the process: the caller queues [`PROBE_REPLY`].
    Answer,
    /// A later probe: never answered twice.
    Unanswered,
    /// A record before any reply: no program reports unasked.
    Ignored,
    Root,
    Child,
    Discarded(Malformed),
}

/// The root record as the status POST carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    pub state: State,
    pub kind: Option<String>,
    pub app: Option<String>,
}

/// What a session's status POST says of its program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub source: Source,
    /// Only where the source is not the title.
    pub activity: Option<Activity>,
    pub root: Option<Root>,
    pub attention: Option<&'static str>,
}

/// A session's program status: the root record, and whether the program is
/// still believed. Child (`id=`) records are parsed and set nothing: the
/// Freeze takes activity from the root record alone (AM113).
#[derive(Debug, Default)]
pub struct ProgramStatus {
    answered: bool,
    first_output: Option<Instant>,
    root: Option<Record>,
    any_record: bool,
    unparsed: bool,
    no_record: bool,
    failures: u32,
}

impl ProgramStatus {
    pub fn output_at(&mut self, now: Instant) {
        self.first_output.get_or_insert(now);
    }

    pub fn see(&mut self, data: &[u8]) -> Seen {
        if !self.answered {
            return match parse(data) {
                Ok(Parsed::Probe) => {
                    self.answered = true;
                    Seen::Answer
                }
                _ => Seen::Ignored,
            };
        }
        match parse(data) {
            Ok(Parsed::Probe) => Seen::Unanswered,
            Ok(Parsed::Record(r)) if r.id.is_none() => {
                self.believed();
                self.unparsed = false;
                self.root = Some(r);
                Seen::Root
            }
            Ok(Parsed::Clear { id: None }) => {
                self.believed();
                self.unparsed = false;
                self.root = None;
                Seen::Root
            }
            Ok(Parsed::Record(_) | Parsed::Clear { .. }) => {
                self.believed();
                Seen::Child
            }
            Err(m) => {
                self.failures += 1;
                if !m.child {
                    self.unparsed = true;
                }
                Seen::Discarded(m)
            }
        }
    }

    fn believed(&mut self) {
        self.any_record = true;
        self.no_record = false;
    }

    /// Whether no record came within [`FIRST_RECORD_WITHIN`] of the first
    /// output although the probe was answered. A later record clears it.
    pub fn no_record(&mut self, now: Instant) -> bool {
        if !self.no_record
            && self.answered
            && !self.any_record
            && self
                .first_output
                .is_some_and(|t| now.duration_since(t) >= FIRST_RECORD_WITHIN)
        {
            self.no_record = true;
        }
        self.no_record
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// The session's activity and where it came from.
    pub fn activity(
        &mut self,
        title: Option<Activity>,
        now: Instant,
    ) -> (Option<Activity>, Source) {
        if self.unparsed {
            return (None, Source::Unknown);
        }
        match self.live_root(now) {
            Some(r) if r.state == State::Working => (Some(Activity::Working), Source::Program),
            Some(_) => (Some(Activity::Idle), Source::Program),
            None => (title, Source::Title),
        }
    }

    fn live_root(&mut self, now: Instant) -> Option<&Record> {
        if self.unparsed || self.no_record(now) {
            return None;
        }
        self.root.as_ref()
    }

    /// What the program says needs the person.
    pub fn attention(&mut self, now: Instant) -> Option<&'static str> {
        self.live_root(now).and_then(attention_of)
    }

    /// What the status POST carries; `None` until the probe was answered,
    /// so a program that never asks changes nothing.
    pub fn report(&mut self, now: Instant) -> Option<Report> {
        if !self.answered {
            return None;
        }
        let (activity, source) = self.activity(None, now);
        let attention = self.attention(now);
        let root = self.live_root(now).map(|r| Root {
            state: r.state,
            kind: r.kind.clone(),
            app: r.app.clone(),
        });
        Some(Report {
            source,
            activity,
            root,
            attention,
        })
    }
}

/// Decision 155's mapping (AM114): `permission` → `permission_prompt`;
/// `question` and a kind-less `blocked` → `question`; `auth` → `sign_in`.
pub fn attention_of(r: &Record) -> Option<&'static str> {
    (r.state == State::Blocked).then_some(match r.kind.as_deref() {
        Some("permission") => "permission_prompt",
        Some("auth") => "sign_in",
        _ => "question",
    })
}

/// The host's circuit breaker: parse failures across its sessions. From
/// [`HOST_MAX_FAILURES`] on, new sessions are not answered, so their
/// program keeps its title glyph and the title gives activity.
#[derive(Debug, Default)]
pub struct Breaker(AtomicU32);

impl Breaker {
    pub fn failed(&self) -> u32 {
        self.0.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Whether a session starting now gets the probe answered.
    pub fn closed(&self) -> bool {
        self.0.load(Ordering::SeqCst) < HOST_MAX_FAILURES
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::TitleObserver;

    fn vectors() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../docs/program-status-vectors.json")).unwrap()
    }

    fn bytes(key: &str) -> Vec<u8> {
        vectors()[key].as_str().unwrap().as_bytes().to_vec()
    }

    fn answered() -> ProgramStatus {
        let mut p = ProgramStatus::default();
        p.output_at(Instant::now());
        assert_eq!(p.see(b"?"), Seen::Answer);
        p
    }

    fn feed(o: &mut TitleObserver, p: &mut ProgramStatus, bytes: &[u8], now: Instant) -> Vec<Seen> {
        o.feed(bytes);
        p.output_at(now);
        o.take_program().iter().map(|d| p.see(d)).collect()
    }

    #[test]
    fn the_captured_probe_is_answered_once_and_the_first_record_is_idle() {
        let t0 = Instant::now();
        let mut o = TitleObserver::new(true, true);
        let mut p = ProgramStatus::default();
        assert_eq!(feed(&mut o, &mut p, &bytes("startup"), t0), [Seen::Answer]);
        assert_eq!(o.current().activity, Some(Activity::Idle), "the glyph");
        assert_eq!(
            feed(&mut o, &mut p, &bytes("first_record"), t0),
            [Seen::Root]
        );
        assert_eq!(o.current().activity, None, "the title lost its glyph");
        assert_eq!(
            p.activity(o.current().activity, t0 + Duration::from_secs(5)),
            (Some(Activity::Idle), Source::Program)
        );
        assert_eq!(
            feed(&mut o, &mut p, b"\x1b]7501;?\x1b\\", t0),
            [Seen::Unanswered]
        );
    }

    #[test]
    fn a_probe_split_byte_by_byte_is_answered_once() {
        let mut o = TitleObserver::new(true, true);
        let mut p = ProgramStatus::default();
        let now = Instant::now();
        let mut seen = Vec::new();
        for b in bytes("startup").iter().chain(b"\x1b]7501;?\x07") {
            seen.extend(feed(&mut o, &mut p, std::slice::from_ref(b), now));
        }
        assert_eq!(seen, [Seen::Answer, Seen::Unanswered]);
    }

    #[test]
    fn an_observer_without_program_status_keeps_no_payloads() {
        let mut o = TitleObserver::new(true, false);
        o.feed(&bytes("startup"));
        assert!(o.take_program().is_empty());
        let mut o = TitleObserver::new(false, true);
        o.feed(&bytes("startup"));
        assert_eq!(o.take_program(), [b"?".to_vec()]);
        assert_eq!(o.current().activity, None, "no titles read");
    }

    #[test]
    fn records_parse_by_the_grammar() {
        let r = |s: &str| parse(s.as_bytes());
        let Ok(Parsed::Record(b)) = r("state=blocked:app=claude-code:kind=permission") else {
            panic!()
        };
        assert_eq!(
            (b.state, b.kind.as_deref(), b.app.as_deref(), b.id),
            (
                State::Blocked,
                Some("permission"),
                Some("claude-code"),
                None
            )
        );
        let Ok(Parsed::Record(k)) = r("state=done:kind=permission:progress=40") else {
            panic!()
        };
        assert_eq!((k.kind, k.progress), (None, None), "only where allowed");
        let Ok(Parsed::Record(m)) = r("state=working:bad pair:X=1:msg=aGk=:progress=101") else {
            panic!()
        };
        assert_eq!(
            (m.state, m.progress, m.msg.as_deref()),
            (State::Working, None, Some(b"hi".as_slice()))
        );
        let Ok(Parsed::Record(bad)) = r("state=working:msg=!!") else {
            panic!()
        };
        assert_eq!(bad.msg, None, "a bad value is a skipped pair");
        assert_eq!(r("state=clear"), Ok(Parsed::Clear { id: None }));
        let unknown = |child| {
            Err(Malformed {
                error: ParseError::UnknownState,
                child,
            })
        };
        assert_eq!(r("state=bogus"), unknown(false));
        assert_eq!(r("state=bogus:id=t1"), unknown(true));
        assert_eq!(
            r("app=x"),
            Err(Malformed {
                error: ParseError::NoState,
                child: false
            })
        );
        assert_eq!(
            r(&format!("state=idle:msg={}", "A".repeat(4100))),
            Err(Malformed {
                error: ParseError::TooLong,
                child: false
            })
        );
        let Ok(Parsed::Record(c)) = r("state=working:app=claude-code:id=task1") else {
            panic!()
        };
        assert_eq!(c.id.as_deref(), Some("task1"));
    }

    #[test]
    fn the_mapping_follows_the_decision() {
        let now = Instant::now();
        let mut p = answered();
        let mut at = |s: &str| {
            p.see(s.as_bytes());
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
        assert_eq!(at("state=clear"), (None, None), "clear: the title's");
    }

    #[test]
    fn a_child_record_never_changes_the_roots_activity() {
        let now = Instant::now();
        let mut p = answered();
        p.see(b"state=done:app=claude-code");
        assert_eq!(p.see(b"state=working:id=a"), Seen::Child);
        assert_eq!(
            p.activity(None, now),
            (Some(Activity::Idle), Source::Program)
        );
        assert_eq!(p.see(b"state=blocked:kind=permission:id=a"), Seen::Child);
        assert_eq!(p.attention(now), None);
        assert_eq!(p.see(b"state=clear:id=a"), Seen::Child);
        assert_eq!(p.report(now).unwrap().root.unwrap().state, State::Done);
    }

    #[test]
    fn no_record_within_two_seconds_falls_back_to_the_title() {
        let t0 = Instant::now();
        let mut o = TitleObserver::new(true, true);
        let mut p = ProgramStatus::default();
        feed(
            &mut o,
            &mut p,
            "\x1b]7501;?\x07\x1b]0;◐ Busy\x07".as_bytes(),
            t0,
        );
        let glyph = o.current().activity;
        assert_eq!(glyph, Some(Activity::Working));
        assert!(!p.no_record(t0 + Duration::from_millis(1900)));
        assert!(p.no_record(t0 + FIRST_RECORD_WITHIN));
        assert_eq!(
            p.activity(glyph, t0 + Duration::from_secs(3)),
            (Some(Activity::Working), Source::Title)
        );
        let report = p.report(t0 + Duration::from_secs(3)).unwrap();
        assert_eq!((report.source, report.root), (Source::Title, None));
        p.see(b"state=done");
        let later = t0 + Duration::from_secs(4);
        assert!(!p.no_record(later), "a late record is believed");
        assert_eq!(
            p.activity(glyph, later),
            (Some(Activity::Idle), Source::Program)
        );
    }

    #[test]
    fn a_record_that_does_not_parse_makes_activity_unknown_not_idle() {
        let now = Instant::now();
        let mut o = TitleObserver::new(true, true);
        let mut p = ProgramStatus::default();
        feed(&mut o, &mut p, &bytes("startup"), now);
        feed(&mut o, &mut p, &bytes("first_record"), now);
        feed(
            &mut o,
            &mut p,
            b"\x1b]7501;state=working:app=claude-code\x07",
            now,
        );
        let seen = feed(&mut o, &mut p, b"\x1b]7501;state=thinking\x07", now);
        assert!(matches!(seen[..], [Seen::Discarded(_)]), "{seen:?}");
        assert_eq!(o.current().activity, None, "no glyph to fall back to");
        assert_eq!(
            p.activity(o.current().activity, now),
            (None, Source::Unknown)
        );
        assert_eq!(p.attention(now), None);
        let report = p.report(now).unwrap();
        assert_eq!((report.activity, report.root), (None, None));
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
    fn the_breaker_opens_at_three_failures() {
        let b = Breaker::default();
        b.failed();
        b.failed();
        assert!(b.closed());
        assert_eq!(b.failed(), HOST_MAX_FAILURES);
        assert!(!b.closed(), "new sessions are not answered");
    }

    #[test]
    fn a_program_that_never_probes_changes_nothing() {
        let now = Instant::now();
        let mut p = ProgramStatus::default();
        p.output_at(now);
        assert_eq!(p.see(b"state=blocked:kind=permission"), Seen::Ignored);
        let later = now + Duration::from_secs(10);
        assert!(!p.no_record(later));
        assert_eq!(p.report(later), None);
        assert_eq!(
            p.activity(Some(Activity::Working), later),
            (Some(Activity::Working), Source::Title)
        );
    }

    #[test]
    fn every_captured_record_maps_as_the_vectors_say() {
        let doc = vectors();
        assert_eq!(doc["reply"].as_str().unwrap().as_bytes(), PROBE_REPLY);
        for v in doc["records"].as_array().unwrap() {
            let case = &v["case"];
            let now = Instant::now();
            let mut p = answered();
            for before in v["after"].as_array().into_iter().flatten() {
                p.see(before.as_str().unwrap().as_bytes());
            }
            p.see(v["raw"].as_str().unwrap().as_bytes());
            let report = p.report(now).unwrap();
            let want = match v["activity"].as_str() {
                Some("working") => Some(Activity::Working),
                Some("idle") => Some(Activity::Idle),
                _ => None,
            };
            let source = match report.source {
                Source::Program => "program",
                Source::Title => "title",
                Source::Unknown => "unknown",
            };
            assert_eq!(source, v["source"], "{case}");
            if report.source != Source::Title {
                assert_eq!(report.activity, want, "{case}");
            }
            assert_eq!(report.attention, v["attention"].as_str(), "{case}");
            let root = report.root.map(
                |r| serde_json::json!({"state": r.state.as_str(), "kind": r.kind, "app": r.app}),
            );
            assert_eq!(root.unwrap_or_default(), v["program_status"], "{case}");
        }
    }
}
