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
    dict(name="desktop-01-notes-provenance", viewport="desktop", route=f"/v/{P}/browse",
         actions=opened(LEAD, SPRINT, GW, BOARD)),
    dict(name="desktop-02-note-start-session", viewport="desktop", route=f"/v/{P}/browse",
         actions=opened(DAILY, LEAD, SPRINT, GW)),
    dict(name="phone-01-notes-vault-root", viewport="phone", route=f"/v/{P}/browse",
         actions=opened(DAILY, LEAD, SPRINT, GW) + [("go", f"/v/{P}/browse")]),
    dict(name="phone-03-note-start-session", viewport="phone", route=f"/v/{P}/note/{GW}"),
    dict(name="desktop-03-vault-switcher", viewport="desktop", route=f"/v/{P}/note/{GW}",
         actions=[("tap", "^personal")]),
    dict(name="desktop-04-health-popover", viewport="desktop", route=f"/v/{P}/note/{BOARD}",
         actions=[("tap", "^Status$")]),
    dict(name="phone-02-place-picker", viewport="phone", route=f"/v/{P}/browse",
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
    dict(name="phone-10-settings-list", viewport="phone", route="/settings"),
    dict(name="phone-11-settings-page-ai-access", viewport="phone", route="/settings",
         actions=[("tap", "^AI access$")]),
]

DESIGN = [
    dict(name="design-system/gallery", viewport="gallery", route="/gallery", settle=2.5),
]

SETS = {"current": CURRENT, "v2": V2, "design": DESIGN}
