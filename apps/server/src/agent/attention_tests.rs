//! Decision 134-9: `needs_attention` — each kind's set and clear, idle never
//! setting it, and `permission_prompt` off for every adapter. The view's
//! field through the real routes is tested in `api.rs`.

use super::store::SessionRecord;
use super::{AgentManager, Hello, Launch, LaunchMeta, McpGrant};

fn hello() -> Hello {
    let providers: Vec<_> = ["claude-code", "opencode", "codex", "shell"]
        .iter()
        .map(|id| serde_json::json!({"id": id, "kind": "cli", "interactions": ["terminal"], "available": true}))
        .collect();
    serde_json::from_value(serde_json::json!({
        "providers": providers,
        "workspaces": ["storm", "other"],
        "workspace_paths": {"storm": "/w/storm", "other": "/w/other", "children/ags_C1": "/c/ags_C1"},
        "max_sessions": 8,
        "mcp_bridge": true,
        "identity": {"state": "verified", "mechanism": "so_peerpidfd", "reason": null},
        "isolation": {"state": "verified", "mechanism": "systemd_units", "reason": null},
        "execution": {"account": "storm-runtime", "isolation": "per-session"},
        "child_workspaces": true,
    }))
    .unwrap()
}

struct Fixture {
    m: AgentManager,
    _dir: tempdir::TempDir,
    generation: u64,
    _link: tokio::sync::mpsc::UnboundedReceiver<super::Envelope>,
}

fn fixture() -> Fixture {
    let dir = tempdir::TempDir::new("storm-attention").unwrap();
    let m = AgentManager::open(dir.path()).unwrap();
    let (generation, link) = m.connect_host("hst_A");
    m.hello("hst_A", hello()).unwrap();
    Fixture {
        m,
        _dir: dir,
        generation,
        _link: link,
    }
}

impl Fixture {
    fn launch(&self, provider: &str) -> SessionRecord {
        self.launch_in(provider, "storm")
    }

    fn launch_in(&self, provider: &str, workspace: &str) -> SessionRecord {
        self.launch_as(provider, workspace, false)
    }

    fn launch_as(&self, provider: &str, workspace: &str, spawn: bool) -> SessionRecord {
        let launch: Launch = serde_json::from_value(serde_json::json!({
            "host_id": "hst_A", "workspace": workspace, "provider": provider,
            "terminal": {"cols": 80, "rows": 24}, "spawn": spawn,
        }))
        .unwrap();
        let grant = McpGrant {
            id: "storm".into(),
            slug: "storm".into(),
            plugin_origin: None,
        };
        let (r, ..) = self
            .m
            .launch("usr_A", launch, LaunchMeta::default(), vec![grant])
            .unwrap();
        self.report(&r.id, serde_json::json!({"status": "running"}));
        r
    }

    fn report(&self, id: &str, body: serde_json::Value) {
        self.m
            .host_status("hst_A", id, &serde_json::from_value(body).unwrap())
            .unwrap();
    }

    fn attention(&self, id: &str) -> Option<&'static str> {
        let r = self.m.get(id).unwrap();
        self.m.needs_attention(&r).unwrap()
    }

    /// A running parent with spawn on and a running child of it.
    fn tree(&self) -> (String, String) {
        let parent = self.launch_as("claude-code", "storm", true).id;
        let mut r = self.m.get(&parent).unwrap();
        r.id = "ags_C1".into();
        r.workspace = "children/ags_C1".into();
        r.status = "running".into();
        self.m
            .store
            .lock()
            .unwrap()
            .insert_spawned(&r, &parent, "a task", "2026-10-25T10:00:00Z")
            .unwrap();
        self.m.report_children_for_tests("hst_A");
        (parent, "ags_C1".into())
    }
}

// ---- no approval (145a) -------------------------------------------------------

/// A-145-1: an act waits for no one, so it never makes its requester need
/// a person, whatever kind it is.
#[test]
fn an_act_never_needs_a_persons_approval() {
    let f = fixture();
    let (parent, child) = f.tree();
    f.m.message_child(&parent, &child, "check the cap")
        .unwrap()
        .unwrap();
    assert_eq!(f.attention(&parent), None);
    assert_eq!(f.attention(&child), None);
    f.m.end_child(&parent, &child).unwrap().unwrap();
    assert_eq!(f.attention(&parent), None);
}

// ---- blocked ----------------------------------------------------------------

#[test]
fn a_blocked_result_needs_attention_until_another_replaces_it_or_the_session_ends() {
    let f = fixture();
    let (parent, child) = f.tree();
    for outcome in ["succeeded", "partial", "failed"] {
        f.m.report_result(&child, outcome, "done", None).unwrap();
        assert_eq!(f.attention(&child), None, "{outcome}");
    }
    f.m.report_result(&child, "blocked", "need a key", None)
        .unwrap();
    assert_eq!(f.attention(&child), Some("blocked"));
    assert_eq!(f.attention(&parent), None, "the child's, not its parent's");
    f.m.report_result(&child, "succeeded", "found it", None)
        .unwrap();
    assert_eq!(f.attention(&child), None, "cleared by a later report");

    f.m.report_result(&child, "blocked", "need a key", None)
        .unwrap();
    f.report(
        &child,
        serde_json::json!({"status": "running", "activity": "idle"}),
    );
    assert_eq!(
        f.attention(&child),
        Some("blocked"),
        "idle clears nothing either"
    );
    f.report(&child, serde_json::json!({"status": "stopped"}));
    assert_eq!(f.attention(&child), None, "cleared by its end");
}

/// 145: a blocked result is its round's: a message that starts the next
/// round clears it.
#[test]
fn a_blocked_result_is_its_rounds_and_the_next_round_clears_it() {
    let f = fixture();
    let (parent, child) = f.tree();
    f.m.report_result(&child, "blocked", "need a key", None)
        .unwrap();
    assert_eq!(f.attention(&child), Some("blocked"));
    assert_eq!(
        f.m.message_child(&parent, &child, "here is the key")
            .unwrap(),
        Ok(2)
    );
    assert_eq!(f.attention(&child), None);
}

// ---- permission_prompt ------------------------------------------------------

const PROMPT: &str = "permission_prompt";

/// 146c: a permission prompt is taken only from an agent whose G-146-P
/// passed (Claude Code and OpenCode), and only through its hook; 134-9's
/// per-adapter list stays empty.
#[test]
fn only_a_gated_agent_reports_a_permission_prompt() {
    assert!(super::attention::PERMISSION_PROMPT_ADAPTERS.is_empty());
    let gated: Vec<&str> = super::attention::GATED_SIGNALS
        .iter()
        .filter(|g| g.signal == PROMPT && g.source == super::events::PROVIDER_HOOK)
        .map(|g| g.provider)
        .collect();
    assert_eq!(gated, ["claude-code", "opencode"]);
    assert_eq!(super::attention::GATED_SIGNALS.len(), gated.len());
    let f = fixture();
    for (provider, taken) in [
        ("claude-code", true),
        ("opencode", true),
        ("codex", false),
        ("shell", false),
    ] {
        let s = f.launch(provider).id;
        f.report(
            &s,
            serde_json::json!({"status": "running", "attention": PROMPT}),
        );
        assert_eq!(f.attention(&s), taken.then_some(PROMPT), "{provider}");
        f.report(
            &s,
            serde_json::json!({"status": "running", "attention": PROMPT,
                "attention_source": "output_pattern"}),
        );
        assert_eq!(f.attention(&s), None, "{provider}: not from a pattern");
    }
}

#[test]
fn a_reported_permission_prompt_is_set_and_cleared_by_its_host() {
    let f = fixture();
    f.m.report_permission_prompts_for_tests(&["claude-code"]);
    let s = f.launch("claude-code").id;
    let other = f.launch("opencode").id;
    f.report(
        &s,
        serde_json::json!({"status": "running", "attention": PROMPT}),
    );
    f.report(
        &other,
        serde_json::json!({"status": "running", "attention": PROMPT}),
    );
    assert_eq!(f.attention(&s), Some(PROMPT));
    assert_eq!(f.attention(&other), None, "only an adapter on the list");

    f.report(
        &s,
        serde_json::json!({"status": "running", "attention": null}),
    );
    assert_eq!(f.attention(&s), None, "cleared");
    f.report(
        &s,
        serde_json::json!({"status": "running", "attention": "idle_prompt"}),
    );
    assert_eq!(f.attention(&s), None, "nothing but a permission prompt");

    f.report(
        &s,
        serde_json::json!({"status": "running", "attention": PROMPT}),
    );
    f.m.disconnect_host("hst_A", f.generation);
    assert_eq!(f.m.get(&s).unwrap().status, "unknown");
    assert_eq!(f.attention(&s), None, "only while it runs");
}

#[test]
fn a_permission_prompt_ends_with_its_session() {
    let f = fixture();
    f.m.report_permission_prompts_for_tests(&["claude-code"]);
    let s = f.launch("claude-code").id;
    f.report(
        &s,
        serde_json::json!({"status": "running", "attention": PROMPT}),
    );
    f.report(
        &s,
        serde_json::json!({"status": "completed", "exit_code": 0, "attention": PROMPT}),
    );
    assert_eq!(f.attention(&s), None);
}

// ---- idle -------------------------------------------------------------------

/// A3: idle is never attention, whatever the adapter, however long, and
/// with the permission prompt mechanism on.
#[test]
fn idle_never_sets_it() {
    let f = fixture();
    f.m.report_permission_prompts_for_tests(&["claude-code", "opencode"]);
    let (parent, child) = f.tree();
    let plain = f.launch_in("opencode", "other").id;
    for s in [&parent, &child, &plain] {
        for activity in ["working", "idle", "idle", "idle"] {
            f.report(
                s,
                serde_json::json!({"status": "running", "activity": activity, "attention": null}),
            );
            assert_eq!(f.attention(s), None, "{s} {activity}");
        }
        f.report(
            s,
            serde_json::json!({"status": "running", "activity": "idle", "title": "Waiting"}),
        );
        assert_eq!(f.attention(s), None, "{s}");
    }
    let mut hello = hello();
    hello.sessions = vec![
        serde_json::from_value(
            serde_json::json!({"id": plain, "status": "running", "activity": "idle"}),
        )
        .unwrap(),
    ];
    f.m.hello("hst_A", hello).unwrap();
    assert_eq!(f.attention(&plain), None, "nor a hello");
}

// ---- program status (decision 155) -------------------------------------------

fn program(state: &str, kind: Option<&str>, activity: &str) -> serde_json::Value {
    serde_json::json!({
        "status": "running", "title": "Claude Code", "activity": activity,
        "activity_source": "program_status",
        "program_status": {"state": state, "kind": kind, "app": "claude-code"},
    })
}

impl Fixture {
    fn program(&self, id: &str) -> Option<(&'static str, Option<&'static str>)> {
        let r = self.m.get(id).unwrap();
        self.m.program_status(&r).map(|p| (p.state, p.kind))
    }

    fn activity(&self, id: &str) -> Option<String> {
        self.m.get(id).unwrap().activity
    }
}

/// Q7 and AM113: each root record's activity, attention and view, for an
/// agent with no gated signal at all and for a Shell running one, as for
/// Claude Code: by capability, never by provider.
#[test]
fn each_program_state_maps_to_activity_and_attention() {
    let f = fixture();
    let rows = [
        (("idle", None), "idle", None, ("idle", None)),
        (("working", None), "working", None, ("working", None)),
        (("done", None), "idle", None, ("done", None)),
        (("error", None), "idle", None, ("error", None)),
        (
            ("blocked", Some("permission")),
            "idle",
            Some(PROMPT),
            ("blocked", Some("permission")),
        ),
        (
            ("blocked", Some("question")),
            "idle",
            Some("question"),
            ("blocked", Some("question")),
        ),
        (
            ("blocked", None),
            "idle",
            Some("question"),
            ("blocked", None),
        ),
        (
            ("blocked", Some("auth")),
            "idle",
            Some("sign_in"),
            ("blocked", Some("auth")),
        ),
        (
            ("blocked", Some("payment")),
            "idle",
            Some("question"),
            ("blocked", None),
        ),
        (("done", Some("permission")), "idle", None, ("done", None)),
    ];
    for provider in ["claude-code", "codex", "shell"] {
        let s = f.launch(provider).id;
        for ((state, kind), activity, attention, view) in rows {
            // The host's own activity is a title's guess here: the record wins.
            let wrong = if activity == "working" {
                "idle"
            } else {
                "working"
            };
            f.report(&s, program(state, kind, wrong));
            let case = format!("{provider} {state} {kind:?}");
            assert_eq!(f.activity(&s).as_deref(), Some(activity), "{case}");
            assert_eq!(f.attention(&s), attention, "{case}");
            assert_eq!(f.program(&s), Some(view), "{case}");
            let r = f.m.get(&s).unwrap();
            let reason = f.m.attention(&r).unwrap().map(|a| a.kind);
            assert_eq!(reason, attention, "{case}");
        }
    }
}

/// Any state but `blocked` clears what a `blocked` set, and so does a
/// report with no record (none yet, `clear`, or one that did not parse).
#[test]
fn a_program_that_is_no_longer_blocked_needs_no_one() {
    let f = fixture();
    let s = f.launch("claude-code").id;
    let no_record = [
        serde_json::json!({"status": "running", "activity": "idle", "program_status": null}),
        serde_json::json!({"status": "running", "activity": "idle"}),
        program("clear", None, "idle"),
        program("thinking", None, "idle"),
    ];
    for (state, kind) in [
        ("idle", None),
        ("working", None),
        ("done", None),
        ("error", None),
    ] {
        for blocked in ["permission", "question", "auth"] {
            f.report(&s, program("blocked", Some(blocked), "idle"));
            assert!(f.attention(&s).is_some(), "{blocked}");
            f.report(&s, program(state, kind, "idle"));
            assert_eq!(f.attention(&s), None, "{blocked} then {state}");
        }
    }
    for body in no_record {
        f.report(&s, program("blocked", Some("permission"), "idle"));
        f.report(&s, body.clone());
        assert_eq!(f.attention(&s), None, "{body}");
        assert_eq!(f.program(&s), None, "{body}");
        assert_eq!(
            f.activity(&s).as_deref(),
            Some("idle"),
            "the title's: {body}"
        );
    }
}

/// 155-R1's safeguard: a record that stopped parsing posts activity null,
/// and nothing here makes it known again.
#[test]
fn an_unknown_activity_stays_unknown_whatever_the_record() {
    let f = fixture();
    let s = f.launch("claude-code").id;
    f.report(&s, program("working", None, "working"));
    for state in ["working", "done", "idle"] {
        let mut body = program(state, None, "idle");
        body["activity"] = serde_json::Value::Null;
        f.report(&s, body);
        assert_eq!(f.activity(&s), None, "{state}");
    }
}

/// A3: `blocked` is the program's own statement; idle, done and error,
/// however long, never need a person.
#[test]
fn a_programs_idle_never_sets_attention() {
    let f = fixture();
    let s = f.launch("claude-code").id;
    for state in ["idle", "done", "error", "idle", "idle"] {
        f.report(&s, program(state, None, "idle"));
        assert_eq!(f.attention(&s), None, "{state}");
    }
}

/// A host from before decision 155 sends no `program_status`: activity and
/// a gated hook's prompt are exactly as before, and a malformed field is
/// no record, never a refused report.
#[test]
fn an_older_host_sees_no_change() {
    let f = fixture();
    let s = f.launch("claude-code").id;
    let codex = f.launch("codex").id;
    for (body, activity, attention) in [
        (
            serde_json::json!({"status": "running", "activity": "working"}),
            Some("working"),
            None,
        ),
        (
            serde_json::json!({"status": "running", "activity": "idle", "attention": PROMPT}),
            Some("idle"),
            Some(PROMPT),
        ),
        (
            serde_json::json!({"status": "running", "activity": "idle", "attention": null}),
            Some("idle"),
            None,
        ),
        (
            serde_json::json!({"status": "running", "activity": "thinking"}),
            None,
            None,
        ),
    ] {
        f.report(&s, body.clone());
        assert_eq!(f.activity(&s).as_deref(), activity, "{body}");
        assert_eq!(
            f.attention(&s),
            attention,
            "{body}: the gated hook, as before"
        );
        assert_eq!(f.program(&s), None, "{body}");
    }
    for malformed in [
        serde_json::json!("blocked"),
        serde_json::json!({"kind": "permission"}),
        serde_json::json!({"state": 3}),
        serde_json::json!([]),
    ] {
        f.report(
            &codex,
            serde_json::json!({"status": "running", "activity": "working", "program_status": malformed}),
        );
        assert_eq!(
            f.activity(&codex).as_deref(),
            Some("working"),
            "{malformed}"
        );
        assert_eq!(f.program(&codex), None, "{malformed}");
    }
}

/// Q10: while a record is live it decides, so a hook's stale prompt after
/// an Esc is not taken; with none, a child's hook is the fallback (146c).
/// A usage limit is the hooks' alone and stays.
#[test]
fn a_live_record_wins_over_a_hook_and_the_hook_is_the_fallback() {
    let f = fixture();
    let (_parent, child) = f.tree();
    f.report(
        &child,
        serde_json::json!({"status": "running", "attention": PROMPT}),
    );
    assert_eq!(
        f.attention(&child),
        Some(PROMPT),
        "the hook, with no record"
    );

    let mut stale = program("idle", None, "idle");
    stale["attention"] = PROMPT.into();
    f.report(&child, stale);
    assert_eq!(f.attention(&child), None, "Esc: the record wins");

    let mut asked = program("blocked", Some("question"), "idle");
    asked["attention"] = PROMPT.into();
    f.report(&child, asked);
    assert_eq!(f.attention(&child), Some("question"), "the record's kind");

    f.m.gate_signals_for_tests(&[super::attention::GatedSignal {
        provider: "claude-code",
        signal: "usage_limit",
        source: "provider_hook",
    }]);
    let mut limited = program("idle", None, "idle");
    limited["attention"] = "usage_limit".into();
    f.report(&child, limited);
    assert_eq!(f.attention(&child), Some("usage_limit"));
}

/// `since` is when the record became what it is, so a client can say how
/// long; a report of the same record keeps it.
#[test]
fn a_records_since_is_its_first_report() {
    let f = fixture();
    let s = f.launch("claude-code").id;
    let since = |f: &Fixture| {
        let r = f.m.get(&s).unwrap();
        f.m.program_status(&r).unwrap().since
    };
    f.report(&s, program("blocked", Some("permission"), "idle"));
    let first = since(&f);
    let r = f.m.get(&s).unwrap();
    assert_eq!(
        f.m.attention(&r).unwrap().unwrap().since.as_deref(),
        Some(first.as_str())
    );
    std::thread::sleep(std::time::Duration::from_millis(1100));
    f.report(&s, program("blocked", Some("permission"), "idle"));
    assert_eq!(since(&f), first, "the same record");
    f.report(&s, program("blocked", Some("question"), "idle"));
    assert_ne!(since(&f), first, "another kind is another record");
}

/// A record is shown only while its session runs, and a hello carries it.
#[test]
fn a_record_lives_with_its_session_and_comes_back_with_hello() {
    let f = fixture();
    let s = f.launch("claude-code").id;
    f.report(&s, program("blocked", Some("auth"), "idle"));
    f.m.disconnect_host("hst_A", f.generation);
    assert_eq!(f.program(&s), None, "unknown while its host is away");
    assert_eq!(f.attention(&s), None);

    let (_generation, _link) = f.m.connect_host("hst_A");
    let mut hello = hello();
    let mut reported = program("blocked", Some("auth"), "idle");
    reported["id"] = s.clone().into();
    hello.sessions = vec![serde_json::from_value(reported).unwrap()];
    f.m.hello("hst_A", hello).unwrap();
    assert_eq!(f.program(&s), Some(("blocked", Some("auth"))));
    assert_eq!(f.attention(&s), Some("sign_in"));

    f.report(
        &s,
        serde_json::json!({"status": "completed", "exit_code": 0,
            "program_status": {"state": "blocked", "kind": "auth"}}),
    );
    assert_eq!(f.program(&s), None, "ended");
    assert_eq!(f.attention(&s), None);
}

/// A child's record reaches its parent as an ordinary `needs_attention`
/// event, the program's word, with its kind as the reason.
#[test]
fn a_childs_record_is_its_parents_event() {
    let f = fixture();
    let (parent, child) = f.tree();
    f.report(&child, program("blocked", Some("auth"), "idle"));
    let e = f.m.events_of_for_tests(&parent).pop().unwrap();
    assert_eq!(
        (
            e.kind.as_str(),
            e.reason.as_deref(),
            e.source.as_str(),
            e.confidence.as_str()
        ),
        (
            "needs_attention",
            Some("sign_in"),
            "program_status",
            "reported"
        )
    );
    let before = f.m.events_of_for_tests(&parent).len();
    f.report(&child, program("blocked", Some("auth"), "idle"));
    assert_eq!(
        f.m.events_of_for_tests(&parent).len(),
        before,
        "once a change"
    );
}
