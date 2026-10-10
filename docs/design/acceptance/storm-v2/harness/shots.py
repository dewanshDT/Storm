"""Which screens the harness captures, per set.

Routes use `{vault:<name>}` and `{note:<vault>/<path>}` placeholders, resolved
against the fixture server at run time (ids are UUIDs minted at boot).

- `current`: the app as it is before Storm v2, for the record (`baseline/`).
- `design`: the `/gallery` route, all three presets side by side.
- `v2`: the approved states, named exactly as the reference screenshots in
  `design_handoff_storm_v2/screenshots/`, filled in slice by slice.
"""

GW = "{note:personal/projects/storm/Gateway spec.md}"
BOARD = "{note:personal/projects/storm/BOARD.md}"
P = "{vault:personal}"
DAILY = "{note:personal/daily/2026-10-07.md}"
SPRINT = "{note:work/Sprint notes.md}"
LEAD = "{note:kit/agents/Storm Lead.md}"


def opened(*notes):
    """Open notes oldest first, so the server's recents end in this order."""
    vaults = {DAILY: P, GW: P, BOARD: P, SPRINT: "{vault:work}", LEAD: "{vault:kit}"}
    # A vault switch adopts the vault before the note opens, so give each open
    # time to reach the server before the next navigation replaces it.
    return [a for n in notes for a in (("go", f"/v/{vaults[n]}/note/{n}"), ("wait", 2.5))]


CURRENT = [
    dict(name="baseline/desktop-note", viewport="desktop", route=f"/v/{P}/note/{GW}"),
    dict(name="baseline/desktop-browse", viewport="desktop", route=f"/v/{P}/browse"),
    dict(name="baseline/desktop-agents", viewport="desktop", route="/agents"),
    dict(name="baseline/desktop-server-settings", viewport="desktop",
         route=f"/v/{P}/settings/server"),
    dict(name="baseline/desktop-client-settings", viewport="desktop",
         route=f"/v/{P}/settings/client"),
    dict(name="baseline/phone-dashboard", viewport="phone", route="/"),
    dict(name="baseline/phone-browse", viewport="phone", route=f"/v/{P}/browse"),
    dict(name="baseline/phone-note", viewport="phone", route=f"/v/{P}/note/{GW}"),
    dict(name="baseline/phone-agents", viewport="phone", route="/agents"),
    dict(name="baseline/phone-server-settings", viewport="phone", route="/settings/server"),
]

V2 = [
    dict(name="desktop-02-note-start-session", viewport="desktop", route=f"/v/{P}/browse",
         actions=opened(DAILY, LEAD, SPRINT, GW)),
    dict(name="phone-01-notes-vault-root", viewport="phone", route=f"/v/{P}/browse",
         actions=opened(DAILY, LEAD, SPRINT, GW) + [("go", f"/v/{P}/browse")]),
    dict(name="phone-03-note-start-session", viewport="phone", route=f"/v/{P}/note/{GW}"),
    dict(name="desktop-03-vault-switcher", viewport="desktop", route=f"/v/{P}/note/{GW}",
         actions=[("tap", "^personal")]),
    dict(name="desktop-04-health-popover", viewport="desktop", route=f"/v/{P}/note/{BOARD}",
         actions=[("tap", "^Status$")]),
    dict(name="phone-02-place-picker", viewport="phone", route=f"/v/{P}/browse", fresh=True,
         actions=[("tap", "^Places$")]),
    # Slice 7: Settings. Seeded through the real API where a page needs state
    # the fixture lacks (an access key, a relay).
    dict(name="desktop-12-settings-this-device", viewport="desktop", route="/settings/device"),
    dict(name="desktop-13-settings-devices-access", viewport="desktop", route="/settings/access",
         actions=[("api", ("POST", "/v1/keys", {"name": "laptop-claude"})),
                  ("go", "/settings/device"), ("go", "/settings/access")]),
    dict(name="desktop-14-settings-vaults", viewport="desktop", route="/settings/vaults"),
    dict(name="desktop-15-settings-ai-access", viewport="desktop", route="/settings/ai",
         actions=[("api", ("PUT", "/v1/config/mcp", {"enabled": True, "writable": False})),
                  ("go", "/settings/device"), ("go", "/settings/ai")]),
    dict(name="desktop-16-settings-integrations", viewport="desktop",
         route="/settings/integrations"),
    dict(name="desktop-17-settings-hosts", viewport="desktop", route="/settings/hosts"),
    dict(name="desktop-18-settings-storage", viewport="desktop", route="/settings/storage"),
    dict(name="desktop-19-settings-connection", viewport="desktop", route="/settings/connection",
         actions=[("api", ("PUT", "/v1/config/relays", {"relays": ["wss://relay.example.net"]})),
                  ("go", "/settings/device"), ("go", "/settings/connection")]),
    dict(name="desktop-20-settings-advanced", viewport="desktop", route="/settings/advanced"),
    dict(name="desktop-21-settings-about-health", viewport="desktop", route="/settings/health"),
    dict(name="desktop-22-settings-terminal", viewport="desktop", route="/settings/terminal",
         settle=1.5),
    dict(name="phone-10-settings-list", viewport="phone", route="/settings"),
    dict(name="phone-11-settings-page-ai-access", viewport="phone", route="/settings",
         actions=[("tap", "^AI access$")]),
    # Slice 6. `step`s move the server's agent state on (agents.py): no
    # host, then hosts with no sessions, then real sessions and writes.
    dict(name="desktop-06c-agents-no-host", viewport="desktop", route="/agents",
         fresh=True),
    dict(step="hosts"),
    dict(name="desktop-06b-agents-first-session", viewport="desktop", route="/agents",
         fresh=True),
    dict(name="desktop-05-new-session-launcher", viewport="desktop",
         route=f"/v/{P}/note/{GW}", fresh=True, actions=[("tap", "^Start session$")],
         settle=2.5),
    dict(name="phone-04-new-session-sheet", viewport="phone", route=f"/v/{P}/note/{GW}",
         fresh=True, actions=[("tap", "^Session$")], settle=2.5),
    dict(step="sessions"),
    # Slice 8, the loop. BOARD's latest agent writer is now the BOARD
    # session, which this device has not seen: opening BOARD marks it seen.
    dict(name="desktop-01-notes-provenance", viewport="desktop", route=f"/v/{P}/browse",
         fresh=True, actions=opened(LEAD, SPRINT, GW, BOARD)),
    # The core loop, end to end: Start session on the note, Launch in the
    # UI, the session opens on Context; then its agent writes (`loop`).
    dict(name="desktop-loop-0-launched-on-context", viewport="desktop",
         route=f"/v/{P}/note/{GW}", fresh=True,
         actions=[("tap", "^Start session$"), ("wait", 1.5), ("tap", "^Workspace: "),
                  ("tap", "^storm$"), ("tap", "^Launch$"), ("wait", 3)],
         settle=2.5),
    dict(step="loop"),
    dict(name="desktop-06-agents-overview", viewport="desktop", route="/agents",
         fresh=True),
    dict(name="desktop-07-session-running", viewport="desktop",
         route="/agents/s/{session:docs-pass}", settle=2.5),
    dict(name="desktop-08-session-context", viewport="desktop",
         route="/agents/s/{session:gateway-spec}?tab=context", settle=2.5),
    dict(name="desktop-09-session-wrote", viewport="desktop",
         route="/agents/s/{session:gateway-spec}?tab=wrote"),
    dict(name="desktop-10-session-about", viewport="desktop",
         route="/agents/s/{session:gateway-spec}?tab=about"),
    dict(name="desktop-11a-session-completed", viewport="desktop",
         route="/agents/s/{session:test-sweep}?tab=context", settle=2.5),
    dict(name="desktop-11b-session-failed", viewport="desktop",
         route="/agents/s/{session:lint-fix}", settle=2.5),
    dict(name="desktop-11c-session-end-confirm", viewport="desktop",
         route="/agents/s/{session:gateway-spec}?tab=context",
         actions=[("tap", "^End$")]),
    dict(name="phone-05-agents-list", viewport="phone", route="/agents", fresh=True),
    dict(name="phone-06-session-running", viewport="phone",
         route="/agents/s/{session:gateway-spec}", settle=2.5),
    dict(name="phone-07-session-details-sheet", viewport="phone",
         route="/agents/s/{session:gateway-spec}", actions=[("tap", "^Details$")]),
    dict(name="phone-08-session-context-note", viewport="phone",
         route="/agents/s/{session:gateway-spec}", fresh=True,
         actions=[("tap", "^▤ Gateway spec$")], settle=2.5),
    dict(name="phone-09-session-ended", viewport="phone",
         route="/agents/s/{session:test-sweep}", fresh=True, settle=2.5),
    # The agent's two notes are unseen here: BOARD (edited since this device
    # opened it) and the new log note, whose folder is opened to show it.
    dict(name="desktop-loop-a-unseen-dots", viewport="desktop", route=f"/v/{P}/note/{GW}",
         fresh=True, actions=[("tap", r"^log($|\n)")], settle=2.5),
    dict(name="desktop-loop-b-edited-by-session", viewport="desktop",
         route=f"/v/{P}/note/{GW}", actions=[("tap", r"^BOARD($|\n)")], settle=2.5),
]

DESIGN = [
    dict(name="design-system/gallery", viewport="gallery", route="/gallery", settle=2.5),
]

SETS = {"current": CURRENT, "v2": V2, "design": DESIGN}
