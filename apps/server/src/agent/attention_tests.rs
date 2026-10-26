//! Decision 134-9: `needs_attention` — each kind's set and clear, idle never
//! setting it, and `permission_prompt` off for every adapter. The view's
//! field through the real routes is tested in `api.rs`.

use super::store::SessionRecord;
use super::store::spawn::{NewRequest, Request};
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

fn stamp(offset_ms: i64) -> String {
    use time::format_description::well_known::Rfc3339;
    (time::OffsetDateTime::now_utc() + time::Duration::milliseconds(offset_ms))
        .format(&Rfc3339)
        .unwrap()
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

    fn message(&self, parent: &str, child: &str) -> Request {
        self.m
            .create_request(
                parent,
                NewRequest::Message {
                    target: child.into(),
                    text: "check the cap".into(),
                },
            )
            .unwrap()
            .unwrap()
    }

    fn raw_request(&self, id: &str, requester: &str, expires_at: &str) {
        self.m.insert_request_for_tests(&Request {
            id: id.into(),
            kind: "start".into(),
            requester: requester.into(),
            target: None,
            payload: r#"{"task":"write the tests"}"#.into(),
            created_at: stamp(-1000),
            expires_at: expires_at.into(),
            state: "pending".into(),
            decided_by_device: None,
            decided_at: None,
            outcome: None,
            reason: None,
            session: None,
        });
    }
}

// ---- approval ---------------------------------------------------------------

#[test]
fn a_pending_request_needs_its_requester_approved_until_it_is_decided() {
    let f = fixture();
    let (parent, child) = f.tree();
    assert_eq!(f.attention(&parent), None);

    let approved = f.message(&parent, &child);
    assert_eq!(f.attention(&parent), Some("approval"));
    assert_eq!(f.attention(&child), None, "only the requester waits");
    f.m.approve_request(&approved.id, "dev_A", false, None)
        .unwrap();
    assert_eq!(f.attention(&parent), None, "cleared by approval");

    let denied = f.message(&parent, &child);
    assert_eq!(f.attention(&parent), Some("approval"));
    f.m.deny_request(&denied.id, "dev_A").unwrap();
    assert_eq!(f.attention(&parent), None, "cleared by denial");

    f.message(&parent, &child);
    assert_eq!(f.attention(&parent), Some("approval"));
    f.m.set_spawn(&parent, false, Some("dev_A"))
        .unwrap()
        .unwrap();
    assert_eq!(f.attention(&parent), None, "cleared by revocation (B2)");
}

#[test]
fn an_expired_request_needs_no_one_even_before_the_sweeper() {
    let f = fixture();
    let (parent, _) = f.tree();
    f.raw_request("agr_LATE", &parent, &stamp(-1));
    assert_eq!(f.attention(&parent), None, "past its expiry");
    f.raw_request("agr_LIVE", &parent, &stamp(60_000));
    assert_eq!(f.attention(&parent), Some("approval"));
    f.raw_request("agr_BAD", &parent, "not a time");
    f.m.expire_requests().unwrap();
    assert_eq!(f.attention(&parent), Some("approval"), "one still waits");
    f.m.deny_request("agr_LIVE", "dev_A").unwrap();
    assert_eq!(f.attention(&parent), None);
}

#[test]
fn an_ended_requester_needs_no_one() {
    let f = fixture();
    let (parent, child) = f.tree();
    f.message(&parent, &child);
    f.report(
        &parent,
        serde_json::json!({"status": "completed", "exit_code": 0}),
    );
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

#[test]
fn a_waiting_request_comes_before_a_blocked_result() {
    let f = fixture();
    let (_, child) = f.tree();
    f.m.report_result(&child, "blocked", "need a key", None)
        .unwrap();
    f.raw_request("agr_CHILD", &child, &stamp(60_000));
    assert_eq!(f.attention(&child), Some("approval"));
    f.m.deny_request("agr_CHILD", "dev_A").unwrap();
    assert_eq!(f.attention(&child), Some("blocked"));
}

// ---- permission_prompt ------------------------------------------------------

const PROMPT: &str = "permission_prompt";

#[test]
fn no_adapter_reports_a_permission_prompt_until_its_gate_passes() {
    assert!(super::attention::PERMISSION_PROMPT_ADAPTERS.is_empty());
    let f = fixture();
    for provider in ["claude-code", "opencode", "codex", "shell"] {
        let s = f.launch(provider).id;
        f.report(
            &s,
            serde_json::json!({"status": "running", "attention": PROMPT}),
        );
        assert_eq!(f.attention(&s), None, "{provider}");
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
