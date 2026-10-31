//! Decisions 144 (its runtime half) and 145 phase 2 at the manager: a
//! round's final message as its result (A-145-6), parking and resuming
//! (A-145-8), usage and duration on round events (A-145-9), the peek's
//! reading (A-145-10, with its scope in `api.rs`), the wake marks and their
//! source scan (A-144-12, A-145-11), and hooks' evidence (A-144-10,
//! A-144-11). Every list is empty in production: each test switches on
//! only what it needs, with the test-only overrides.

use tokio::sync::mpsc::UnboundedReceiver;

use super::attention::{GatedSignal, IDLE, PERMISSION_PROMPT, USAGE_LIMIT};
use super::events::{self, ChildEvent, GatedWake, PROVIDER_HOOK, Usage};
use super::park::{MAX_PARKED, PARK_AFTER};
use super::round::{FinalMessage, RESULT_SOURCES, ResultSource};
use super::store::spawn::{MAX_DETAILS_BYTES, MAX_SUMMARY_BYTES, StartPayload};
use super::{AgentManager, Command, Envelope, Hello, Launch, LaunchMeta, McpGrant, wake};

const UUID: &str = "80d0cf8c-8176-43b4-8206-7908d74d1387";

fn hello() -> Hello {
    let providers: Vec<_> = ["claude-code", "opencode"]
        .iter()
        .map(|id| serde_json::json!({"id": id, "kind": "cli", "interactions": ["terminal"], "available": true, "mcp": true}))
        .collect();
    serde_json::from_value(serde_json::json!({
        "providers": providers,
        "workspaces": ["storm", "other"],
        "workspace_paths": {"storm": "/w/storm", "other": "/w/other"},
        "max_sessions": 16,
        "mcp_bridge": true,
        "resume": ["claude-code", "opencode"],
        "identity": {"state": "verified", "mechanism": "so_peerpidfd", "reason": null},
        "isolation": {"state": "verified", "mechanism": "systemd_units", "reason": null},
    }))
    .unwrap()
}

/// What passing each gate would switch on, for Claude Code: its hooks'
/// idle, usage limit and permission prompt.
const HOOKS: &[GatedSignal] = &[
    GatedSignal {
        provider: "claude-code",
        signal: IDLE,
        source: PROVIDER_HOOK,
    },
    GatedSignal {
        provider: "claude-code",
        signal: USAGE_LIMIT,
        source: PROVIDER_HOOK,
    },
    GatedSignal {
        provider: "claude-code",
        signal: PERMISSION_PROMPT,
        source: PROVIDER_HOOK,
    },
];

const RESULTS: &[ResultSource] = &[ResultSource {
    provider: "claude-code",
    source: PROVIDER_HOOK,
}];

const WAKES: &[GatedWake] = &[GatedWake {
    provider: "claude-code",
    mechanism: "async_rewake",
}];

struct Fixture {
    m: AgentManager,
    _dir: tempdir::TempDir,
    link: std::sync::Mutex<UnboundedReceiver<Envelope>>,
}

fn fixture() -> Fixture {
    let dir = tempdir::TempDir::new("storm-lifecycle").unwrap();
    let m = AgentManager::open(dir.path()).unwrap();
    let (_, link) = m.connect_host("hst_A");
    m.hello("hst_A", hello()).unwrap();
    m.verify_isolation_for_tests("hst_A");
    Fixture {
        m,
        _dir: dir,
        link: std::sync::Mutex::new(link),
    }
}

impl Fixture {
    fn launch(&self, write_vault: Option<&str>) -> String {
        let launch: Launch = serde_json::from_value(serde_json::json!({
            "host_id": "hst_A", "workspace": "storm", "provider": "claude-code",
            "terminal": {"cols": 80, "rows": 24}, "spawn": true,
        }))
        .unwrap();
        let grant = McpGrant {
            id: "storm".into(),
            slug: "storm".into(),
            plugin_origin: None,
        };
        let meta = LaunchMeta {
            write_vault_id: write_vault.map(Into::into),
            ..LaunchMeta::default()
        };
        let (r, ..) = self.m.launch("usr_A", launch, meta, vec![grant]).unwrap();
        self.report(&r.id, serde_json::json!({"status": "running"}));
        r.id
    }

    fn report(&self, id: &str, body: serde_json::Value) {
        self.m
            .host_status("hst_A", id, &serde_json::from_value(body).unwrap())
            .unwrap();
    }

    /// A child started by `parent`'s act, running, on its conversation.
    fn child(&self, parent: &str, task: &str) -> String {
        let payload = StartPayload {
            task: task.into(),
            ..StartPayload::default()
        };
        let child = self.m.start_child(parent, payload, None).unwrap().unwrap();
        self.report(
            &child.id,
            serde_json::json!({"status": "running", "harness_session": UUID}),
        );
        child.id
    }

    fn sent(&self) -> Vec<Command> {
        let mut link = self.link.lock().unwrap();
        std::iter::from_fn(|| link.try_recv().ok())
            .map(|e| e.command)
            .collect()
    }

    fn events(&self, parent: &str) -> Vec<ChildEvent> {
        self.m.events_of_for_tests(parent)
    }

    fn last(&self, parent: &str) -> ChildEvent {
        self.events(parent).pop().unwrap()
    }

    fn final_message(&self, id: &str, text: &str, usage: Option<Usage>) {
        self.m
            .final_message(
                "hst_A",
                id,
                &FinalMessage {
                    text: text.into(),
                    source: None,
                    usage,
                },
            )
            .unwrap();
    }

    /// The child's round reported and its hooks saying it is idle, past
    /// [`PARK_AFTER`].
    fn idle_past_park(&self, id: &str) {
        self.report(
            id,
            serde_json::json!({"status": "running", "activity": "idle", "activity_source": "provider_hook"}),
        );
        self.m
            .advance_clock_for_tests(PARK_AFTER + std::time::Duration::from_secs(1));
    }
}

fn usage(input: u64, output: u64) -> Usage {
    Usage {
        input_tokens: Some(input),
        output_tokens: Some(output),
        ..Usage::default()
    }
}

// ---- A-145-6 -----------------------------------------------------------------

#[test]
fn a_145_6_a_gated_final_message_is_the_rounds_result_and_never_an_event() {
    const CANARY: &str = "CANARY-145-6";
    let f = fixture();
    assert_eq!(
        RESULT_SOURCES,
        [ResultSource {
            provider: "claude-code",
            source: PROVIDER_HOOK
        }],
        "only Claude Code's final message has passed its gate"
    );
    f.m.gate_result_sources_for_tests(&[]);
    let p = f.launch(None);
    let c = f.child(&p, "count the tests");
    let before = f.events(&p).len();

    // Ungated: dropped, nothing written.
    f.final_message(&c, &format!("{CANARY} ungated"), Some(usage(10, 5)));
    assert!(f.m.result_of(&c).unwrap().is_none());
    assert_eq!(f.events(&p).len(), before, "no gate, no event");

    f.m.gate_result_sources_for_tests(RESULTS);
    f.final_message(&c, &format!("{CANARY} there are 12 tests"), None);
    let result = f.m.result_of(&c).unwrap().unwrap();
    assert_eq!((result.round, result.outcome.as_str()), (1, "succeeded"));
    assert_eq!(result.summary, format!("{CANARY} there are 12 tests"));
    let e = f.last(&p);
    assert_eq!(
        (
            e.kind.as_str(),
            e.reason.as_deref(),
            e.source.as_str(),
            e.confidence.as_str()
        ),
        (
            events::REPORTED,
            Some("succeeded"),
            PROVIDER_HOOK,
            "reported"
        )
    );
    let all = serde_json::to_string(&f.events(&p)).unwrap();
    assert!(!all.contains(CANARY), "{all}");

    // A round the child reported itself keeps its own report.
    f.m.message_child(&p, &c, "now count the docs")
        .unwrap()
        .unwrap();
    f.m.report_result(&c, "partial", "half the docs", None)
        .unwrap();
    f.final_message(&c, &format!("{CANARY} final words"), None);
    let round2 = f.m.result_of(&c).unwrap().unwrap();
    assert_eq!(
        (
            round2.round,
            round2.outcome.as_str(),
            round2.summary.as_str()
        ),
        (2, "partial", "half the docs")
    );

    // Bounded as report_result's.
    f.m.message_child(&p, &c, "and the rest").unwrap().unwrap();
    let long = "x".repeat(MAX_DETAILS_BYTES * 2);
    f.final_message(&c, &long, None);
    let round3 = f.m.result_of(&c).unwrap().unwrap();
    assert_eq!(round3.round, 3);
    assert_eq!(round3.summary.len(), MAX_SUMMARY_BYTES);
    assert_eq!(round3.details.unwrap().len(), MAX_DETAILS_BYTES);

    // Only for a spawned session, on its own host.
    let q = f.launch(None);
    f.final_message(&q, "a root's words", None);
    assert!(f.m.result_of(&q).unwrap().is_none());
    assert!(matches!(
        f.m.final_message(
            "hst_B",
            &c,
            &FinalMessage {
                text: "forged".into(),
                ..FinalMessage::default()
            }
        ),
        Err(super::AgentError::NotYours)
    ));
    // Another source is no source.
    f.m.message_child(&p, &c, "one more").unwrap().unwrap();
    f.m.final_message(
        "hst_A",
        &c,
        &FinalMessage {
            text: "from the output".into(),
            source: Some("output_pattern".into()),
            usage: None,
        },
    )
    .unwrap();
    assert_eq!(f.m.result_of(&c).unwrap().unwrap().round, 3);
}

/// Without a gated source the opening prompt's `report_result` path is
/// what reports a round, as before.
#[test]
fn a_145_6_without_a_gated_source_report_result_still_reports() {
    let f = fixture();
    let p = f.launch(None);
    let c = f.child(&p, "count the tests");
    f.final_message(&c, "ignored", None);
    f.m.report_result(&c, "succeeded", "12 tests", None)
        .unwrap();
    let result = f.m.result_of(&c).unwrap().unwrap();
    assert_eq!(result.summary, "12 tests");
    assert_eq!(f.last(&p).source, events::STORM);
}

// ---- A-145-8 -----------------------------------------------------------------

#[test]
fn a_145_8_a_reported_idle_child_parks_and_a_message_resumes_it() {
    let f = fixture();
    let p = f.launch(None);
    let c = f.child(&p, "count the tests");
    f.sent();

    // Without a gated idle signal nothing ever parks.
    f.m.report_result(&c, "succeeded", "12 tests", None)
        .unwrap();
    f.idle_past_park(&c);
    assert!(f.m.park_idle_children().unwrap().is_empty());

    f.m.gate_signals_for_tests(HOOKS);
    f.report(
        &c,
        serde_json::json!({"status": "running", "activity": "idle", "activity_source": "provider_hook"}),
    );
    f.m.advance_clock_for_tests(PARK_AFTER / 2);
    assert!(f.m.park_idle_children().unwrap().is_empty(), "not yet");
    f.m.advance_clock_for_tests(PARK_AFTER);
    assert_eq!(f.m.park_idle_children().unwrap(), std::slice::from_ref(&c));
    assert_eq!(f.sent(), [Command::Park { session: c.clone() }]);
    assert!(f.m.park_idle_children().unwrap().is_empty(), "asked once");

    // The host stops it and says so.
    f.report(
        &c,
        serde_json::json!({"status": "parked", "harness_session": UUID}),
    );
    let r = f.m.get(&c).unwrap();
    assert_eq!(r.status, "parked");
    assert!(!r.is_ended());
    assert_eq!(r.harness_session.as_deref(), Some(UUID), "its conversation");
    let e = f.last(&p);
    assert_eq!(
        (e.kind.as_str(), e.round, e.source.as_str()),
        (events::PARKED, Some(1), events::HOST_PROCESS)
    );
    assert_eq!(f.m.health_activity(&r), None, "no process, no activity");

    // A message resumes it in the same session, from its own conversation.
    let round = f.m.message_child(&p, &c, "now the docs").unwrap().unwrap();
    assert_eq!(round, 2);
    assert!(f.m.resume_parked(&c, None).unwrap());
    match f.sent().as_slice() {
        [
            Command::Start {
                session,
                resume,
                resumed_from,
                child_of,
                workspace,
                ..
            },
        ] => {
            assert_eq!(session, &c);
            assert_eq!(resume.as_deref(), Some(UUID));
            assert_eq!(resumed_from.as_deref(), Some(c.as_str()));
            assert_eq!(child_of.as_deref(), Some(p.as_str()));
            assert_eq!(workspace, "storm", "its parent's, as its first start");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(f.m.get(&c).unwrap().status, "starting");
    f.report(&c, serde_json::json!({"status": "running"}));
    assert_eq!(f.last(&p).kind, events::STARTED);
    assert_eq!(f.m.round_of(&c).unwrap(), 2);
    assert_eq!(f.m.children_of(&p).unwrap().len(), 1, "no new child");
}

#[test]
fn a_145_8_parked_children_free_the_live_cap_and_count_toward_their_own() {
    use super::store::spawn::MAX_CHILDREN;
    let f = fixture();
    f.m.gate_signals_for_tests(HOOKS);
    let p = f.launch(None);
    let children: Vec<String> = (0..MAX_CHILDREN)
        .map(|n| f.child(&p, &format!("task {n}")))
        .collect();
    let fourth = StartPayload {
        task: "a fourth".into(),
        ..StartPayload::default()
    };
    let refused = f.m.start_child(&p, fourth.clone(), None).unwrap();
    assert_eq!(refused.unwrap_err().cap, Some("children"));

    for c in &children {
        f.m.report_result(c, "succeeded", "done", None).unwrap();
        f.report(
            c,
            serde_json::json!({"status": "running", "activity": "idle", "activity_source": "provider_hook"}),
        );
    }
    f.m.advance_clock_for_tests(PARK_AFTER + std::time::Duration::from_secs(1));
    assert_eq!(f.m.park_idle_children().unwrap().len(), children.len());
    for c in &children {
        f.report(c, serde_json::json!({"status": "parked"}));
    }
    let started = f.m.start_child(&p, fourth, None).unwrap();
    assert!(
        started.is_ok(),
        "{started:?}: a 4th task with 3 parked starts"
    );
    assert_eq!(f.m.store.lock().unwrap().parked_on("hst_A").unwrap(), 3);

    // The tree's total still counts them: root, 3 parked, 1 live, then one
    // more is 6, and the next is over.
    f.child(&p, "a fifth");
    let r = f.m.get(&p).unwrap();
    let cap = f.m.store.lock().unwrap().start_cap(&r, 0).unwrap();
    assert_eq!(cap, Some("tree"));
    assert_eq!(MAX_PARKED, 12);
}

#[test]
fn a_145_8_a_parked_child_ends_for_good_without_its_host() {
    let f = fixture();
    let p = f.launch(None);
    let c = f.child(&p, "count the tests");
    f.m.park_for_tests("hst_A", &c);
    assert_eq!(f.m.get(&c).unwrap().status, "parked");
    f.sent();
    f.m.end_child(&p, &c).unwrap().unwrap();
    let r = f.m.get(&c).unwrap();
    assert_eq!(r.status, "stopped");
    assert!(f.sent().is_empty(), "no host runs it: nothing to send");
    assert_eq!(f.last(&p).kind, events::ENDED);
    // Not resumed once ended.
    assert!(!f.m.resume_parked(&c, None).unwrap());
}

#[test]
fn a_145_8_parked_survives_a_lost_link_and_a_restart_and_only_storm_parks() {
    let f = fixture();
    let p = f.launch(None);
    let c = f.child(&p, "count the tests");
    // A `parked` Storm never asked for is a stop.
    let d = f.child(&p, "another");
    f.report(&d, serde_json::json!({"status": "parked"}));
    assert_eq!(f.m.get(&d).unwrap().status, "stopped");

    f.m.park_for_tests("hst_A", &c);
    let (generation, _link) = f.m.connect_host("hst_A");
    f.m.disconnect_host("hst_A", generation);
    assert_eq!(f.m.get(&c).unwrap().status, "parked", "no process to lose");
    let (_, _link) = f.m.connect_host("hst_A");
    f.m.hello("hst_A", hello()).unwrap();
    assert_eq!(
        f.m.get(&c).unwrap().status,
        "parked",
        "a host with nothing to report of it does not lose it"
    );
    let Fixture { m, _dir: dir, .. } = f;
    drop(m);
    let m = AgentManager::open(dir.path()).unwrap();
    assert_eq!(m.get(&c).unwrap().status, "parked", "nor does a restart");
}

#[test]
fn a_parked_childs_input_is_refused() {
    let f = fixture();
    let p = f.launch(None);
    let c = f.child(&p, "count the tests");
    f.m.park_for_tests("hst_A", &c);
    assert!(f.m.input(&c, b"hello").is_err());
}

// ---- A-145-9 -----------------------------------------------------------------

#[test]
fn a_145_9_usage_is_on_a_rounds_events_only_when_the_agent_said() {
    let f = fixture();
    f.m.gate_result_sources_for_tests(RESULTS);
    let p = f.launch(None);
    let c = f.child(&p, "count the tests");

    f.m.report_result(&c, "succeeded", "12 tests", None)
        .unwrap();
    let e = f.last(&p);
    assert_eq!(e.round, Some(1));
    assert!(e.duration_ms.is_some_and(|d| d >= 0), "{e:?}");
    assert_eq!(e.usage, None, "the agent said nothing of it");
    let text = serde_json::to_string(&e).unwrap();
    assert!(!text.contains("usage"), "absent, never guessed: {text}");

    f.m.message_child(&p, &c, "now the docs").unwrap().unwrap();
    f.final_message(&c, "8 docs", Some(usage(1200, 340)));
    let e = f.last(&p);
    assert_eq!((e.kind.as_str(), e.round), (events::REPORTED, Some(2)));
    assert_eq!(e.usage, Some(usage(1200, 340)));
    assert!(e.duration_ms.is_some());

    // An end carries its round, and that round's usage if any was said.
    f.report(
        &c,
        serde_json::json!({"status": "completed", "exit_code": 0}),
    );
    let e = f.last(&p);
    assert_eq!((e.kind.as_str(), e.round), (events::ENDED, Some(2)));
    assert_eq!(e.usage, Some(usage(1200, 340)));

    // Non-round kinds carry none of it.
    let started = f
        .events(&p)
        .into_iter()
        .find(|e| e.kind == events::STARTED)
        .unwrap();
    assert_eq!(
        (started.round, started.duration_ms, started.usage),
        (None, None, None)
    );
}

// ---- A-145-10 ----------------------------------------------------------------

#[test]
fn a_145_10_the_peek_is_stripped_and_bounded() {
    let f = fixture();
    let p = f.launch(None);
    let c = f.child(&p, "count the tests");
    let mut out = Vec::new();
    for n in 0..300 {
        out.extend_from_slice(format!("\x1b[1;32mline {n}\x1b[0m\r\n").as_bytes());
    }
    out.extend_from_slice(b"\x1b]0;a title\x07progress 10%\rprogress 99%\r\n");
    out.extend_from_slice(b"\x1b[?25lthe end\x1b[K\n");
    f.m.host_output("hst_A", &c, 0, &out).unwrap();
    let tail = f.m.output_tail(&c, 200);
    assert_eq!(tail.len(), 200);
    assert_eq!(tail[tail.len() - 1], "the end");
    assert_eq!(tail[tail.len() - 2], "progress 99%");
    assert_eq!(tail[tail.len() - 3], "line 299");
    assert!(tail.iter().all(|l| !l.contains('\x1b')), "{tail:?}");
    assert_eq!(f.m.output_tail(&c, 3).len(), 3);
    assert!(f.m.output_tail("ags_nobody", 10).is_empty());
}

/// Round 2: `read_output` gave a Claude Code child's spinner, never its
/// words. Claude Code 2.1.314 draws by cursor position, with no newline
/// between frames; these are its bytes for two turns (a stand-in model's
/// answers) at 100×30. The peek is its screen.
#[test]
fn a_claude_code_childs_peek_is_its_screen_not_its_spinner() {
    let bytes = include_bytes!("claude_code_screen.bin");
    let lines = super::screen_lines(bytes, (100, 30), 200);
    let text = lines.join("\n");
    for words in [
        "ECHO:first-answer-words",
        "● first-answer-words",
        "ECHO:second-answer-words",
        "● second-answer-words",
    ] {
        assert!(lines.iter().any(|l| l.contains(words)), "{words}: {text}");
    }
    assert!(
        !text.contains("Gesticulating"),
        "a spinner frame since redrawn: {text}"
    );
    assert!(!text.contains('\x1b'));
    let first = lines.iter().position(|l| l.contains("● first")).unwrap();
    let second = lines.iter().position(|l| l.contains("● second")).unwrap();
    assert!(first < second, "{text}");

    let f = fixture();
    let p = f.launch(None);
    let c = f.child(&p, "say it");
    f.m.host_output("hst_A", &c, 0, bytes).unwrap();
    let tail = f.m.output_tail(&c, 200);
    assert!(
        tail.iter().any(|l| l.contains("● second-answer-words")),
        "{tail:?}"
    );
}

// ---- wake marks (144c, 145d) ---------------------------------------------------

#[test]
fn a_wake_mark_goes_only_to_a_gated_agents_host_and_once_per_mark() {
    let f = fixture();
    let p = f.launch(None);
    let c = f.child(&p, "count the tests");
    f.sent();
    assert!(events::WAKE.is_empty());
    f.m.report_result(&c, "succeeded", "done", None).unwrap();
    assert_eq!(f.m.sync_wakes().unwrap(), 0, "no wake has passed its gate");
    assert!(f.sent().is_empty());

    f.m.gate_wake_for_tests(WAKES);
    assert_eq!(f.m.unread_mark(&p).unwrap().0, wake::EVENTS);
    assert_eq!(f.m.unread_mark(&c).unwrap().0, wake::MESSAGE);
    f.m.sync_wakes().unwrap();
    let marks = |sent: Vec<Command>| -> Vec<(String, String, i64)> {
        sent.into_iter()
            .filter_map(|c| match c {
                Command::Wake {
                    session,
                    kind,
                    mark,
                } => Some((session, kind, mark)),
                _ => None,
            })
            .collect()
    };
    let last_seq = f.last(&p).seq;
    assert_eq!(
        marks(f.sent()),
        [
            (p.clone(), "events".to_string(), last_seq),
            (c.clone(), "message".to_string(), 0)
        ]
    );
    assert_eq!(f.m.sync_wakes().unwrap(), 0, "each mark once");

    // The parent acknowledges: nothing unread.
    f.m.store.lock().unwrap().ack_events(&p, last_seq).unwrap();
    f.m.sync_wakes().unwrap();
    assert_eq!(marks(f.sent()), [(p.clone(), "events".to_string(), 0)]);

    // A message: the child's mark is its number, until it reads it.
    f.m.message_child(&p, &c, "next").unwrap().unwrap();
    f.m.sync_wakes().unwrap();
    assert_eq!(marks(f.sent()), [(c.clone(), "message".to_string(), 1)]);
    f.m.store.lock().unwrap().read_messages(&c, None).unwrap();
    f.m.sync_wakes().unwrap();
    assert_eq!(marks(f.sent()), [(c.clone(), "message".to_string(), 0)]);

    // A host that comes back, still running both, is told again.
    let (_, _link) = f.m.connect_host("hst_A");
    let mut back = hello();
    back.sessions = [&p, &c]
        .iter()
        .map(|id| {
            serde_json::from_value(serde_json::json!({"id": id, "status": "running"})).unwrap()
        })
        .collect();
    f.m.hello("hst_A", back).unwrap();
    assert_eq!(f.m.sync_wakes().unwrap(), 2);
}

// ---- A-144-10, A-144-11 (hooks' evidence) ---------------------------------------

#[test]
fn a_144_10_a_gated_hooks_idle_and_usage_limit_are_provider_hook_evidence() {
    let f = fixture();
    let p = f.launch(None);
    let c = f.child(&p, "count the tests");
    let before = f.events(&p).len();
    for body in [
        serde_json::json!({"status": "running", "attention": USAGE_LIMIT, "attention_source": "provider_hook"}),
        serde_json::json!({"status": "running", "activity": "idle", "activity_source": "provider_hook"}),
    ] {
        f.report(&c, body);
    }
    assert_eq!(f.events(&p).len(), before, "ungated: no event");
    assert_eq!(
        f.m.health_activity(&f.m.get(&c).unwrap()).as_deref(),
        Some("unknown")
    );

    f.m.gate_signals_for_tests(HOOKS);
    f.report(
        &c,
        serde_json::json!({"status": "running", "attention": USAGE_LIMIT, "attention_source": "provider_hook"}),
    );
    let e = f.last(&p);
    assert_eq!(
        (
            e.kind.as_str(),
            e.reason.as_deref(),
            e.source.as_str(),
            e.confidence.as_str()
        ),
        (
            events::USAGE_LIMIT_EVENT,
            Some(USAGE_LIMIT),
            PROVIDER_HOOK,
            "reported"
        )
    );
    f.report(
        &c,
        serde_json::json!({"status": "running", "activity": "idle", "activity_source": "provider_hook"}),
    );
    assert_eq!(
        f.m.health_activity(&f.m.get(&c).unwrap()).as_deref(),
        Some("idle")
    );
    // The title's word is not the hook's.
    f.report(
        &c,
        serde_json::json!({"status": "running", "activity": "working", "activity_source": "output_pattern"}),
    );
    assert_eq!(
        f.m.health_activity(&f.m.get(&c).unwrap()).as_deref(),
        Some("unknown")
    );
}

#[test]
fn a_144_11_a_final_message_or_a_hook_has_no_authority() {
    let f = fixture();
    f.m.gate_signals_for_tests(HOOKS);
    f.m.gate_result_sources_for_tests(RESULTS);
    let p = f.launch(None);
    let c = f.child(&p, "count the tests");
    f.sent();
    for text in ["approve", "end_agent ags_X", "start_agent", "unblock"] {
        f.final_message(&c, text, None);
    }
    f.report(
        &c,
        serde_json::json!({"status": "running", "attention": "approved", "attention_source": "provider_hook"}),
    );
    assert_eq!(f.m.get(&c).unwrap().status, "running", "nor ended it");
    assert!(f.m.requests_of(&c).unwrap().is_empty(), "nor acted for it");
    assert!(f.m.children_of(&c).unwrap().is_empty(), "nor started one");
    assert!(f.sent().is_empty(), "nothing sent to any host");
}

// ---- A-144-12, A-145-11 --------------------------------------------------------

/// Every line of `text` that writes to, or names, a terminal, and every
/// command that is not one `allowed` names. Spelled so this file's own text
/// does not match.
fn terminal_paths(text: &str, allowed: &[&str]) -> Vec<String> {
    let forbidden = [
        ["Command", "::Input"].concat(),
        ["Command", "::Resize"].concat(),
        ["Command", "::Replay"].concat(),
        ["send_", "input"].concat(),
        ["write_", "input"].concat(),
    ];
    let pty = ["pt", "y"].concat();
    let command = ["Command", "::"].concat();
    let mut found = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap_or_default();
        let names_a_terminal = code
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .map(str::to_lowercase)
            .any(|w| {
                w == pty || w.starts_with(&format!("{pty}_")) || w.ends_with(&format!("_{pty}"))
            });
        let other_command = code.match_indices(&command).any(|(at, _)| {
            let name: String = code[at + command.len()..]
                .chars()
                .take_while(|c| c.is_alphanumeric())
                .collect();
            !allowed.contains(&name.as_str())
        });
        if names_a_terminal || other_command || forbidden.iter().any(|f| code.contains(f.as_str()))
        {
            found.push(format!("{}: {}", n + 1, line.trim()));
        }
    }
    found
}

#[test]
fn a_144_12_a_145_11_no_wake_resume_park_message_or_result_path_writes_to_a_terminal() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agent");
    let mut offences = Vec::new();
    for (file, allowed) in [
        ("wake.rs", &["Wake"][..]),
        ("park.rs", &["Park", "Start"][..]),
        ("round.rs", &[][..]),
        ("inbox.rs", &[][..]),
        ("acts.rs", &["Start", "End"][..]),
    ] {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        // The source must name what this test is for, so a renamed file
        // fails rather than passing on nothing.
        assert!(text.len() > 1000, "{file} is read");
        for hit in terminal_paths(&text, allowed) {
            offences.push(format!("{file}:{hit}"));
        }
    }
    assert!(
        offences.is_empty(),
        "a 145 path reaches a terminal:\n{}",
        offences.join("\n")
    );
}

#[test]
fn a_144_12_a_145_11_the_scan_finds_a_planted_write() {
    for planted in [
        "self.send(&r.host_id, Command::Input { session, data })",
        "let c = Command::Resize { session, cols, rows };",
        "host.write_input(&session, b\"go\")",
        "pty_write(fd, b\"wake\")",
        "let cmd = Command::End { session };",
    ] {
        assert_eq!(terminal_paths(planted, &["Wake"]).len(), 1, "{planted}");
    }
    assert!(
        terminal_paths(
            "let wake = Command::Wake { session, kind, mark };",
            &["Wake"]
        )
        .is_empty()
    );
}

// ---- Phase 1 follow-up: the write vault ----------------------------------------

#[test]
fn a_child_inherits_its_parents_write_vault_only_when_it_launched_with_one() {
    let f = fixture();
    let writer = f.launch(Some("vlt_W"));
    let reader = f.launch(None);
    let w = f.child(&writer, "write the notes");
    let r = f.child(&reader, "read the notes");
    let store = f.m.store.lock().unwrap();
    assert_eq!(
        store.launch(&w).unwrap().unwrap().write_vault_id.as_deref(),
        Some("vlt_W")
    );
    assert_eq!(
        store.grant(&w, "storm").unwrap(),
        Some(("storm".to_string(), true))
    );
    assert_eq!(store.launch(&r).unwrap().unwrap().write_vault_id, None);
    assert_eq!(
        store.grant(&r, "storm").unwrap(),
        Some(("storm".to_string(), false)),
        "nothing wider than the parent's"
    );
}
