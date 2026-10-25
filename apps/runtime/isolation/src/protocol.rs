//! The helper's wire format (136c): one JSON object per connection, one line
//! each way. A request is an `op` and exactly that operation's fields, every
//! one a string. Nothing else parses: an unknown operation, a field the
//! operation does not have (a program, arguments, an environment, a uid), a
//! repeated key or a non-string value is refused before anything is looked
//! at.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::{self, Deserializer, MapAccess, Visitor};

/// A request is far smaller; anything larger is refused unread.
pub const MAX_REQUEST: usize = 16 * 1024;
const MAX_VALUE: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Op {
    Lease,
    Release,
    Prepare,
    Clean,
    Start,
    Signal,
    End,
    ChildCreate,
    ChildExport,
    ChildImport,
    ChildDispose,
    SelfTest,
    AgentJob,
}

impl Op {
    pub const ALL: [Op; 13] = [
        Op::Lease,
        Op::Release,
        Op::Prepare,
        Op::Clean,
        Op::Start,
        Op::Signal,
        Op::End,
        Op::ChildCreate,
        Op::ChildExport,
        Op::ChildImport,
        Op::ChildDispose,
        Op::SelfTest,
        Op::AgentJob,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Op::Lease => "lease",
            Op::Release => "release",
            Op::Prepare => "prepare",
            Op::Clean => "clean",
            Op::Start => "start",
            Op::Signal => "signal",
            Op::End => "end",
            Op::ChildCreate => "child_create",
            Op::ChildExport => "child_export",
            Op::ChildImport => "child_import",
            Op::ChildDispose => "child_dispose",
            Op::SelfTest => "self_test",
            Op::AgentJob => "agent_job",
        }
    }

    /// Exactly the fields the operation takes. `start` names a provider by
    /// id; the program, its arguments and its environment come from the
    /// operator's configuration.
    pub fn fields(self) -> &'static [&'static str] {
        match self {
            Op::Lease | Op::End => &["session"],
            Op::Release | Op::Prepare | Op::Clean => &["session", "user"],
            Op::Start => &["session", "user", "provider", "workspace"],
            Op::Signal => &["session", "signal"],
            Op::ChildCreate => &["parent", "child", "workspace"],
            Op::ChildExport | Op::ChildDispose => &["child"],
            Op::ChildImport => &["child", "parent"],
            Op::SelfTest => &[],
            // 136g: an agent by id and what to do; the recipe is the host's.
            Op::AgentJob => &["agent", "action"],
        }
    }

    fn from_name(name: &str) -> Option<Op> {
        Op::ALL.into_iter().find(|op| op.name() == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub op: Op,
    pub fields: BTreeMap<&'static str, String>,
}

impl Request {
    pub fn get(&self, field: &str) -> &str {
        self.fields.get(field).map(String::as_str).unwrap_or("")
    }
}

/// Why a request was refused. The code is what the host is told and what
/// the audit line names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    PeerNotHost,
    TooLarge,
    Malformed,
    UnknownOp,
    UnexpectedField(String),
    MissingField(&'static str),
    BadValue(&'static str),
    BadSession,
    NotPoolUser,
    Root,
    HostAccount,
    NotLeased,
    LeasedElsewhere,
    BadProvider,
    PathOutsideRoots,
    BadSignal,
    NotBuilt,
    /// `start` without a terminal, or a descriptor with anything else.
    NoTerminal,
    UnexpectedDescriptor,
    NotATerminal,
    PoolExhausted,
    AlreadyLeased,
    AlreadyStarted,
    NotStarted,
    /// Another operation for the same session is in flight.
    Busy,
    /// The provider's program is not on this host.
    ProviderUnavailable,
    BadLaunch,
    /// A resume whose predecessor still holds its lease.
    PredecessorRunning,
    /// A resume whose predecessor kept no conversation.
    NothingKept,
    /// An agent Storm does not install, or an action it does not do.
    BadAgent,
    /// The host has no install identity yet (136g): `on` has to run again.
    NoInstallIdentity,
    /// The workspace, a session directory or systemd failed the operation.
    Failed,
}

impl Refusal {
    pub fn code(&self) -> &'static str {
        match self {
            Refusal::PeerNotHost => "peer_not_host",
            Refusal::TooLarge => "too_large",
            Refusal::Malformed => "malformed",
            Refusal::UnknownOp => "unknown_op",
            Refusal::UnexpectedField(_) => "unexpected_field",
            Refusal::MissingField(_) => "missing_field",
            Refusal::BadValue(_) => "bad_value",
            Refusal::BadSession => "bad_session",
            Refusal::NotPoolUser => "not_pool_user",
            Refusal::Root => "root",
            Refusal::HostAccount => "host_account",
            Refusal::NotLeased => "not_leased",
            Refusal::LeasedElsewhere => "leased_elsewhere",
            Refusal::BadProvider => "bad_provider",
            Refusal::PathOutsideRoots => "path_outside_roots",
            Refusal::BadSignal => "bad_signal",
            Refusal::NotBuilt => "not_built",
            Refusal::NoTerminal => "no_terminal",
            Refusal::UnexpectedDescriptor => "unexpected_descriptor",
            Refusal::NotATerminal => "not_a_terminal",
            Refusal::PoolExhausted => "pool_exhausted",
            Refusal::AlreadyLeased => "already_leased",
            Refusal::AlreadyStarted => "already_started",
            Refusal::NotStarted => "not_started",
            Refusal::Busy => "busy",
            Refusal::ProviderUnavailable => "provider_unavailable",
            Refusal::BadLaunch => "bad_launch",
            Refusal::PredecessorRunning => "predecessor_running",
            Refusal::NothingKept => "nothing_kept",
            Refusal::BadAgent => "bad_agent",
            Refusal::NoInstallIdentity => "no_install_identity",
            Refusal::Failed => "failed",
        }
    }

    /// A short, printable detail for the audit line: a field's name, never
    /// a value from the request.
    pub fn detail(&self) -> Option<String> {
        match self {
            Refusal::UnexpectedField(f) => Some(printable(f)),
            Refusal::MissingField(f) | Refusal::BadValue(f) => Some((*f).to_string()),
            _ => None,
        }
    }
}

fn printable(s: &str) -> String {
    let clean: String = s
        .chars()
        .take(32)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '?'
            }
        })
        .collect();
    if clean.is_empty() { "?".into() } else { clean }
}

/// A JSON object whose keys are unique and whose values are strings.
struct StrictObject(Vec<(String, String)>);

impl<'de> serde::Deserialize<'de> for StrictObject {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = StrictObject;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object of strings")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<StrictObject, A::Error> {
                let mut out: Vec<(String, String)> = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    if out.iter().any(|(k, _)| *k == key) {
                        return Err(de::Error::custom("repeated key"));
                    }
                    let value: String = map.next_value()?;
                    out.push((key, value));
                }
                Ok(StrictObject(out))
            }
        }
        d.deserialize_map(V)
    }
}

pub fn parse(bytes: &[u8]) -> Result<Request, Refusal> {
    if bytes.len() > MAX_REQUEST {
        return Err(Refusal::TooLarge);
    }
    let StrictObject(pairs) = serde_json::from_slice(bytes).map_err(|_| Refusal::Malformed)?;
    let op = match pairs.iter().find(|(k, _)| k == "op") {
        Some((_, name)) => Op::from_name(name).ok_or(Refusal::UnknownOp)?,
        None => return Err(Refusal::MissingField("op")),
    };
    let mut fields = BTreeMap::new();
    for (key, value) in pairs {
        if key == "op" {
            continue;
        }
        let Some(&field) = op.fields().iter().find(|f| **f == key) else {
            return Err(Refusal::UnexpectedField(key));
        };
        if value.is_empty() || value.len() > MAX_VALUE || value.contains('\0') {
            return Err(Refusal::BadValue(field));
        }
        fields.insert(field, value);
    }
    for &field in op.fields() {
        if !fields.contains_key(field) {
            return Err(Refusal::MissingField(field));
        }
    }
    Ok(Request { op, fields })
}

pub fn ok(op: Op) -> String {
    ok_with(op, serde_json::Map::new())
}

/// A success with the operation's own answer: `lease`'s user, `start`'s
/// leader pid, `end`'s exit status or signal.
pub fn ok_with(op: Op, mut fields: serde_json::Map<String, serde_json::Value>) -> String {
    fields.insert("ok".into(), true.into());
    fields.insert("op".into(), op.name().into());
    format!("{}\n", serde_json::Value::Object(fields))
}

pub fn refused(r: &Refusal) -> String {
    format!("{}\n", serde_json::json!({"ok": false, "error": r.code()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operation_parses_with_exactly_its_fields() {
        for op in Op::ALL {
            let mut obj = serde_json::Map::new();
            obj.insert("op".into(), op.name().into());
            for f in op.fields() {
                obj.insert((*f).into(), "x".into());
            }
            let req = parse(
                serde_json::Value::Object(obj.clone())
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
            assert_eq!(req.op, op);
            assert_eq!(req.fields.len(), op.fields().len());
            for f in op.fields() {
                let mut short = obj.clone();
                short.remove(*f);
                assert_eq!(
                    parse(serde_json::Value::Object(short).to_string().as_bytes()),
                    Err(Refusal::MissingField(f)),
                    "{} without {f}",
                    op.name()
                );
            }
        }
    }

    #[test]
    fn a_request_naming_a_program_or_anything_extra_is_refused() {
        for extra in [
            "program", "command", "argv", "args", "env", "uid", "path", "cwd", "exec",
        ] {
            let line = format!(
                r#"{{"op":"start","session":"ags_A","user":"storm-s001","provider":"shell","workspace":"/w/a","{extra}":"/bin/sh"}}"#
            );
            assert_eq!(
                parse(line.as_bytes()),
                Err(Refusal::UnexpectedField(extra.into()))
            );
        }
        assert_eq!(
            parse(br#"{"op":"lease","session":"ags_A","user":"root"}"#),
            Err(Refusal::UnexpectedField("user".into())),
            "lease picks the user itself"
        );
    }

    #[test]
    fn unknown_and_malformed_requests_are_refused() {
        assert_eq!(
            parse(br#"{"op":"exec","session":"ags_A"}"#),
            Err(Refusal::UnknownOp)
        );
        assert_eq!(
            parse(br#"{"op":"Lease","session":"ags_A"}"#),
            Err(Refusal::UnknownOp)
        );
        assert_eq!(
            parse(br#"{"session":"ags_A"}"#),
            Err(Refusal::MissingField("op"))
        );
        for bad in [
            &b"not json"[..],
            br#"["lease"]"#,
            br#"{"op":"lease","session":1}"#,
            br#"{"op":"lease","session":["ags_A"]}"#,
            br#"{"op":"lease","session":"ags_A","session":"ags_B"}"#,
            br#"{"op":"lease","op":"self_test","session":"ags_A"}"#,
            b"\xff\xfe",
        ] {
            assert_eq!(
                parse(bad),
                Err(Refusal::Malformed),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        assert_eq!(
            parse(br#"{"op":"lease","session":""}"#),
            Err(Refusal::BadValue("session"))
        );
        assert_eq!(
            parse(b"{\"op\":\"lease\",\"session\":\"a\\u0000b\"}"),
            Err(Refusal::BadValue("session"))
        );
        assert_eq!(parse(&vec![b' '; MAX_REQUEST + 1]), Err(Refusal::TooLarge));
    }

    #[test]
    fn the_audit_detail_never_carries_a_value() {
        let r = parse(br#"{"op":"end","session":"ags_A","rm -rf /":"x"}"#).unwrap_err();
        assert_eq!(r.detail().as_deref(), Some("rm??rf??"));
        assert_eq!(
            refused(&r),
            "{\"error\":\"unexpected_field\",\"ok\":false}\n"
        );
    }
}
