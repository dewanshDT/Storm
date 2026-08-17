//! Model Context Protocol — tools over the same operations REST uses.
//!
//! Every tool here is params → [`crate::ops`] → structured content. Nothing in
//! this file reaches for `Db` or the filesystem directly, and nothing
//! reimplements a rule that already exists: vault resolution, FTS sanitising
//! and the not-found cases all come from `ops`, so MCP and REST cannot drift
//! apart. That is the whole point of `docs/storm-mcp.md`'s Principle, and the
//! reason `ops` exists at all.
//!
//! **Read-only unless switched on.** The write tools are filtered out of the
//! router entirely when the server is in read-only mode, so an agent is never
//! shown a tool it cannot use — better than letting it choose one and refusing,
//! and impossible to forget in a single tool's body. See [`WRITE_TOOLS`].
//!
//! Notes are addressed by `vault + note_id`, never by a path — the same
//! contract the REST API already has. Responses do carry the vault-relative
//! path, which is structure a caller needs, but an absolute filesystem path
//! must never appear in one; `mcp_e2e.py` asserts it.

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;

use crate::api::Shared;
use crate::auth::authz::Actor;

/// How the endpoint is configured at startup.
#[derive(Debug, Clone)]
pub struct McpOptions {
    /// `Host` headers to accept, empty meaning any.
    pub allowed_hosts: Vec<String>,
}

/// Tools that change the vault.
///
/// Named here rather than inferred, so adding a write tool without deciding it
/// is a write is not possible: `every_write_tool_is_listed` fails if the router
/// grows one that is missing from this list, and it would otherwise be served
/// to a read-only client.
pub const WRITE_TOOLS: &[&str] = &["create_note", "update_note", "delete_note"];

/// Which `Host` headers the MCP endpoint should accept.
///
/// rmcp defaults this to loopback only, as DNS-rebinding protection. That
/// default is wrong for Storm and fails in a way that is hard to read: the
/// server is reached at its LAN address, so every request from another machine
/// would be refused with nothing pointing at the `Host` header as the cause.
///
/// So the bind address is added to the list. When binding to a wildcard the
/// list is emptied, because the header will carry whichever address the client
/// used and we cannot enumerate those. That is an acceptable trade here and not
/// a general one: the rebinding attack it defends against is a browser being
/// tricked into posting to the server, which the bearer token already refuses —
/// and this whole design is LAN-only per decision 4.
pub fn allowed_hosts(host: &str, port: u16) -> Vec<String> {
    if matches!(host, "0.0.0.0" | "::" | "[::]") {
        return Vec::new();
    }
    vec![
        "localhost".into(),
        format!("localhost:{port}"),
        host.to_string(),
        format!("{host}:{port}"),
    ]
}

tokio::task_local! {
    /// The authenticated caller of the MCP request currently being served.
    ///
    /// **Read by [`service`]'s factory, never by a tool.** rmcp serves a
    /// stateless request by building the handler on the request task and then
    /// running it inside a `tokio::spawn` (`streamable_http_server/tower.rs`:
    /// `get_service()` at the top of `handle_post`, `tokio::spawn` a few lines
    /// later). A task-local does **not** cross a spawn, so a tool that tried to
    /// read this would find nothing. The factory reads it while still on the
    /// request task and moves the value into the handler, which is what carries
    /// the identity across the spawn — by ownership, not by ambient state.
    ///
    /// That is also why it cannot leak between concurrent requests: each
    /// request is its own task with its own scope, and each handler owns a
    /// separate `Actor` from the moment it is built.
    static MCP_ACTOR: Actor;
}

/// Puts the authenticated actor where [`service`]'s factory can find it.
///
/// Layered **inside** `require_auth`, so `Extension<Actor>` is already set. A
/// request that somehow arrives without one is served with no identity, and the
/// vault tools refuse — see [`Storm::actor`].
pub async fn scope_actor(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    match request.extensions().get::<Actor>().cloned() {
        Some(actor) => MCP_ACTOR.scope(actor, next.run(request)).await,
        None => next.run(request).await,
    }
}

/// The tower service to nest at `/mcp`.
pub fn service(
    state: Shared,
    options: McpOptions,
) -> StreamableHttpService<Storm, LocalSessionManager> {
    // Built with `with_*` rather than a struct literal: the config is
    // `#[non_exhaustive]`, which is rmcp saying it expects to grow fields.
    let config = StreamableHttpServerConfig::default()
        // The 2026-07-28 revision removed sessions (SEP-2567), so requests
        // negotiating it are served statelessly whatever this says. Setting it
        // false keeps older clients on the same path rather than giving Storm
        // two behaviours to reason about — and with read-only tools there is no
        // per-session state worth keeping anyway.
        .with_legacy_session_mode(false)
        // Plain JSON responses instead of an SSE stream. Every tool here is a
        // single request/response with no progress to report, and it keeps the
        // e2e test a plain POST rather than a stream parser.
        .with_json_response(true)
        .with_allowed_hosts(options.allowed_hosts);

    StreamableHttpService::new(
        // A fresh handler per request, which is what lets the write switch take
        // effect immediately: the mode is read here, not captured at startup.
        move || {
            let writable = state
                .mcp_writable
                .load(std::sync::atomic::Ordering::Relaxed);
            // Read here, on the request task, because this closure is the last
            // point before rmcp spawns the handler. `try_with` rather than
            // `get`: this same factory is also called to build a throwaway
            // service for tool-schema validation, which has no request scope
            // and needs no identity.
            let actor = MCP_ACTOR.try_with(Clone::clone).ok();
            Ok(Storm::new(state.clone(), writable, actor))
        },
        Arc::new(LocalSessionManager::default()),
        config,
    )
}

// ---- tool parameters ---------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct VaultParams {
    /// The vault's id, as returned by `list_vaults`.
    pub vault: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NoteParams {
    /// The vault's id, as returned by `list_vaults`.
    pub vault: String,
    /// The note's id. Notes are addressed by id, never by file path.
    pub note_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// The vault's id, as returned by `list_vaults`.
    pub vault: String,
    /// Words to look for. Punctuation is quoted for you, so `foo-bar` is safe.
    pub query: String,
    /// How many hits to return. Defaults to 20, capped at 500.
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RelatedParams {
    pub vault: String,
    pub note_id: String,
    /// How many tag-related notes to return. Defaults to 20.
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct VersionParams {
    pub vault: String,
    pub note_id: String,
    /// A version number from `get_note_history`.
    pub version: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RecentParams {
    /// How many notes to return across all vaults. Defaults to 20.
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateParams {
    pub vault: String,
    /// Where the note goes, relative to the vault, ending in `.md` — for
    /// example `Projects/Ideas.md`. Folders are created as needed.
    pub path: String,
    /// The note's full markdown. Frontmatter is optional; Storm adds `id` and
    /// `created` itself.
    pub content: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateParams {
    pub vault: String,
    pub note_id: String,
    /// The note's complete new markdown. This replaces the body — read the note
    /// first and send back the whole thing, not a fragment.
    pub content: String,
    /// The `version` from the `get_note` you edited. The server merges against
    /// it, so a note changed on a phone in the meantime is reconciled rather
    /// than overwritten.
    pub base_version: i64,
}

// ---- the handler -------------------------------------------------------

/// Holds only the shared state.
///
/// No `tool_router` field, though the SDK's example carries one: `#[tool_handler]`
/// resolves the router through the static `Self::tool_router()` that
/// `#[tool_router]` generates, so a field would never be read. Storm is
/// constructed per request by the service factory anyway, and with read-only
/// tools there is nothing per-connection worth keeping.
#[derive(Clone)]
pub struct Storm {
    state: Shared,
    /// Who is calling — the same `Actor` a REST request would resolve.
    ///
    /// Captured when the handler is built, *not* read per tool: rmcp runs the
    /// handler on a spawned task, and this value crossing that boundary by
    /// ownership is what keeps concurrent requests from seeing each other's
    /// identity.
    actor: Option<Actor>,
    /// Whether this request may change the vault.
    ///
    /// Filters the tool router rather than gating each tool's body, so a
    /// read-only server does not *advertise* `delete_note` at all. An agent
    /// cannot pick a tool it was never shown, which is a better answer than
    /// letting it try and refusing — and it makes the refusal impossible to
    /// forget in one tool's body.
    writable: bool,
}

/// Renders an operation's result as a tool result.
///
/// Two details here are the spec's, not preference.
///
/// **The value must be a JSON object.** "Structured content is returned as a
/// JSON object in the `structuredContent` field" — so a bare array or string is
/// out of spec even though `rmcp` types the field as any `Value` and will
/// happily send one. Callers returning a list pass a key to wrap it under, and
/// those keys are the REST envelopes' own (`vaults`, `hits`, `tags`), so the
/// two surfaces read the same.
///
/// **A missing vault is a tool error, not a protocol error.** The spec draws
/// the line at self-correction: tool execution errors "contain actionable
/// feedback that language models can use to self-correct", and clients *should*
/// pass them to the model, while protocol errors are for malformed requests and
/// clients need not surface them at all. "No such note" is exactly the former —
/// an agent holding a stale id should be told so and retry. So anything `ops`
/// reports as a 4xx comes back as `isError: true` carrying the same message
/// REST would give; only a genuine server fault becomes a protocol error.
fn respond<T: serde::Serialize>(
    result: crate::api::ApiResult<T>,
    key: Option<&str>,
) -> Result<CallToolResult, ErrorData> {
    match result {
        Ok(value) => {
            let json = serde_json::to_value(&value)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            let object = match key {
                Some(key) => serde_json::json!({ key: json }),
                None => json,
            };
            debug_assert!(
                object.is_object(),
                "structuredContent must be a JSON object; pass a key to wrap {object}"
            );
            Ok(CallToolResult::structured(object))
        }
        Err(e) if e.0.is_server_error() => Err(ErrorData::internal_error(e.1, None)),
        Err(e) => Ok(CallToolResult::structured_error(
            serde_json::json!({ "error": e.1 }),
        )),
    }
}

/// A result that is already object-shaped.
fn respond_object<T: serde::Serialize>(
    result: crate::api::ApiResult<T>,
) -> Result<CallToolResult, ErrorData> {
    respond(result, None)
}

#[tool_router]
impl Storm {
    pub fn new(state: Shared, writable: bool, actor: Option<Actor>) -> Self {
        Self {
            state,
            writable,
            actor,
        }
    }

    /// The authenticated caller, or a refusal.
    ///
    /// `None` means the handler was built outside a request scope. That should
    /// not happen for a tool call — `/mcp` sits behind the session tier — so it
    /// is refused rather than defaulted. **Fail closed:** a default here would
    /// be an unauthenticated caller reaching a vault the moment the policy
    /// stops being permissive, which is the bypass this slice exists to remove.
    fn actor(&self) -> Result<&Actor, ErrorData> {
        self.actor.as_ref().ok_or_else(|| {
            ErrorData::internal_error(
                "this MCP request carries no authenticated identity".to_string(),
                None,
            )
        })
    }

    /// The tools this request is allowed to see and call.
    fn tools(&self) -> ToolRouter<Self> {
        let mut router = Self::tool_router();
        if !self.writable {
            for name in WRITE_TOOLS {
                router.remove_route(name);
            }
        }
        router
    }

    #[tool(
        description = "List the vaults on this Storm server, with their ids, names and note counts. Call this first — every other tool needs a vault id."
    )]
    async fn list_vaults(&self) -> Result<CallToolResult, ErrorData> {
        respond(
            crate::ops::list_vaults(&self.state, self.actor()?).await,
            Some("vaults"),
        )
    }

    #[tool(
        description = "Describe one vault: its note count, its folders, and the description its owner wrote for it."
    )]
    async fn get_vault(
        &self,
        Parameters(VaultParams { vault }): Parameters<VaultParams>,
    ) -> Result<CallToolResult, ErrorData> {
        respond_object(crate::ops::get_vault(&self.state, self.actor()?, &vault).await)
    }

    #[tool(
        description = "Full-text search one vault, returning matching notes with a highlighted snippet from each. The fastest way to find a note."
    )]
    async fn search(
        &self,
        Parameters(SearchParams {
            vault,
            query,
            limit,
        }): Parameters<SearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            crate::ops::search(
                &self.state,
                self.actor()?,
                &vault,
                &query,
                limit.unwrap_or(20),
            )
            .await,
            Some("hits"),
        )
    }

    #[tool(description = "Read one note in full: its markdown content and its metadata.")]
    async fn get_note(
        &self,
        Parameters(NoteParams { vault, note_id }): Parameters<NoteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        respond_object(crate::ops::get_note(&self.state, self.actor()?, &vault, &note_id).await)
    }

    #[tool(
        description = "Notes related to this one: those linking to it, and those sharing its tags. Every relation is exact and says why it is one — there is no semantic guessing here."
    )]
    async fn get_related_notes(
        &self,
        Parameters(RelatedParams {
            vault,
            note_id,
            limit,
        }): Parameters<RelatedParams>,
    ) -> Result<CallToolResult, ErrorData> {
        respond_object(
            crate::ops::related(
                &self.state,
                self.actor()?,
                &vault,
                &note_id,
                limit.unwrap_or(20),
            )
            .await,
        )
    }

    #[tool(description = "Every tag in a vault with how many notes carry it.")]
    async fn list_tags(
        &self,
        Parameters(VaultParams { vault }): Parameters<VaultParams>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            crate::ops::list_tags(&self.state, self.actor()?, &vault).await,
            Some("tags"),
        )
    }

    #[tool(
        description = "Notes opened most recently, across every vault. Good for 'what was I working on'."
    )]
    async fn recent_notes(
        &self,
        Parameters(RecentParams { limit }): Parameters<RecentParams>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            crate::ops::recents(&self.state, self.actor()?, limit.unwrap_or(20)).await,
            Some("recents"),
        )
    }

    #[tool(
        description = "A note's revision history: version numbers, when each was written and by which device. Content is not included — fetch one with get_note_version."
    )]
    async fn get_note_history(
        &self,
        Parameters(NoteParams { vault, note_id }): Parameters<NoteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            crate::ops::note_history(&self.state, self.actor()?, &vault, &note_id).await,
            Some("versions"),
        )
    }

    #[tool(description = "The full text of one earlier revision of a note.")]
    async fn get_note_version(
        &self,
        Parameters(VersionParams {
            vault,
            note_id,
            version,
        }): Parameters<VersionParams>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            crate::ops::note_version(&self.state, self.actor()?, &vault, &note_id, version).await,
            Some("content"),
        )
    }

    // ---- writes --------------------------------------------------------
    //
    // Present only when the server is in read-write mode; see `tools()`.

    #[tool(
        description = "Create a new note. Fails if the path is already taken. The note is stamped `source: ai` in its frontmatter so it is obvious later who wrote it."
    )]
    async fn create_note(
        &self,
        Parameters(CreateParams {
            vault,
            path,
            content,
        }): Parameters<CreateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        // Stamped here rather than in `ops`, because "an agent wrote this" is
        // something only this caller knows — a note created from the phone must
        // not carry it. `set_scalars` splices a single line and passes every
        // other byte through, so a user's YAML keeps its order and comments.
        let stamped = crate::frontmatter::set_scalars(&content, &[("source", "ai")]);
        respond_object(
            crate::ops::create_note(&self.state, self.actor()?, &vault, &path, &stamped).await,
        )
    }

    #[tool(
        description = "Replace a note's content. Requires the base_version you read, so a note edited elsewhere is merged rather than overwritten. If the result says merged or conflict, the returned content is the server's — re-read before editing again."
    )]
    async fn update_note(
        &self,
        Parameters(UpdateParams {
            vault,
            note_id,
            content,
            base_version,
        }): Parameters<UpdateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        respond_object(
            crate::ops::update_note(
                &self.state,
                self.actor()?,
                &vault,
                &note_id,
                base_version,
                &content,
                Some("mcp"),
            )
            .await,
        )
    }

    #[tool(
        description = "Delete a note. Storm has no trash: the file is removed from the vault immediately. Only the server's version history still holds the text."
    )]
    async fn delete_note(
        &self,
        Parameters(NoteParams { vault, note_id }): Parameters<NoteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            crate::ops::delete_note(&self.state, self.actor()?, &vault, &note_id).await,
            Some("seq"),
        )
    }
}

#[tool_handler(router = self.tools())]
impl ServerHandler for Storm {
    fn get_info(&self) -> ServerInfo {
        // Field-by-field on a default, because these models are
        // `#[non_exhaustive]` — see the config above.
        let mut info = ServerInfo::default();
        // The SDK's default, deliberately, rather than the newest constant it
        // knows. `ProtocolVersion::LATEST` is 2025-11-25 — rmcp knows
        // 2026-07-28 but has not promoted it, which is the same Tier 2 caveat
        // `docs/storm-mcp.md` records. Following the default means Storm moves
        // when the SDK does, rather than announcing a revision its own
        // transport treats as provisional.
        info.protocol_version = ProtocolVersion::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::new("storm", env!("CARGO_PKG_VERSION"));
        // The instructions follow the mode, because they are the first thing
        // the model reads: telling it the tools are read-only when they are not
        // would be worse than saying nothing.
        info.instructions = Some(
            if self.writable {
                "Storm is a self-hosted markdown notes server. Notes live in vaults and are \
                 addressed by vault id and note id — never by file path. Start with \
                 list_vaults, then search to find notes and get_note to read one. You may \
                 also create, update and delete notes: always read a note before updating it \
                 and send back its base_version, so a change made on another device is \
                 merged rather than overwritten. There is no trash — a deleted note is gone \
                 from the vault immediately."
            } else {
                "Storm is a self-hosted markdown notes server. Notes live in vaults and are \
                 addressed by vault id and note id — never by file path. Start with \
                 list_vaults, then search to find notes and get_note to read one. These \
                 tools are read-only: this server does not allow changes."
            }
            .into(),
        );
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_mode_does_not_even_advertise_the_write_tools() {
        // Filtering the router rather than gating each tool's body means an
        // agent is never shown a tool it cannot use — and that the refusal
        // cannot be forgotten in one tool.
        let read_only = Storm::tool_router();
        let listed: Vec<String> = read_only
            .list_all()
            .into_iter()
            .map(|t| t.name.into())
            .collect();
        for name in WRITE_TOOLS {
            assert!(
                listed.contains(&name.to_string()),
                "{name} should exist in the full router"
            );
        }

        let mut filtered = Storm::tool_router();
        for name in WRITE_TOOLS {
            filtered.remove_route(name);
        }
        for name in WRITE_TOOLS {
            assert!(
                !filtered.has_route(name),
                "{name} must be gone when read-only"
            );
        }
        assert_eq!(
            filtered.list_all().len(),
            listed.len() - WRITE_TOOLS.len(),
            "only the write tools should have been removed"
        );
    }

    #[test]
    fn every_tool_that_changes_the_vault_is_named_in_write_tools() {
        // The list is hand-maintained, so this is what stops a new write tool
        // from being served to a read-only client because someone forgot to add
        // it. Heuristic but deliberate: any tool whose name begins with a verb
        // that changes something must be declared.
        const MUTATING_PREFIXES: [&str; 5] = ["create", "update", "delete", "move", "append"];
        for tool in Storm::tool_router().list_all() {
            let name: String = tool.name.into();
            if MUTATING_PREFIXES.iter().any(|p| name.starts_with(p)) {
                assert!(
                    WRITE_TOOLS.contains(&name.as_str()),
                    "{name} looks like a write tool but is not in WRITE_TOOLS, \
                     so a read-only server would serve it"
                );
            }
        }
    }

    #[test]
    fn a_concrete_bind_address_is_allowed_with_and_without_its_port() {
        // The failure this prevents: rmcp's default list is loopback only, so
        // a phone or laptop reaching the server at its LAN address gets every
        // request refused, and nothing in the error mentions the Host header.
        let hosts = allowed_hosts("192.168.91.51", 8484);
        assert!(hosts.contains(&"192.168.91.51:8484".to_string()));
        assert!(hosts.contains(&"192.168.91.51".to_string()));
        assert!(hosts.contains(&"localhost:8484".to_string()));
    }

    #[test]
    fn a_wildcard_bind_allows_any_host() {
        // Binding to 0.0.0.0 means the Host header carries whichever address
        // the client used, which cannot be enumerated ahead of time. Empty is
        // rmcp's "allow all".
        for wildcard in ["0.0.0.0", "::", "[::]"] {
            assert!(
                allowed_hosts(wildcard, 8484).is_empty(),
                "{wildcard} should not restrict hosts"
            );
        }
    }
}
