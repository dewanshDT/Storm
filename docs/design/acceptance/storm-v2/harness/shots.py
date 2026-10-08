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
    dict(name="desktop-04-health-popover", viewport="desktop", route=f"/v/{P}/note/{BOARD}",
         actions=[("tap", "^Status$")]),
    dict(name="phone-02-place-picker", viewport="phone", route=f"/v/{P}/browse", fresh=True,
         actions=[("tap", "^Places$")]),
] + [
    # Slice 6. `step`s move the server's agent state on (agents.py): no
    # host, then hosts with no sessions, then real sessions and writes.
    dict(name="desktop-06c-agents-no-host", viewport="desktop", route="/agents",
         fresh=True),
    dict(step="hosts"),
    dict(name="desktop-06b-agents-first-session", viewport="desktop", route="/agents",
         fresh=True),
    dict(name="desktop-05-new-session-launcher", viewport="desktop", route="/agents",
         fresh=True, actions=[("tap", "^Start from Gateway spec$")], settle=2.5),
    dict(name="phone-04-new-session-sheet", viewport="phone", route="/agents",
         fresh=True, actions=[("tap", "^Start from ▶  Gateway spec$")], settle=2.5),
    dict(step="sessions"),
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
]

DESIGN = [
    dict(name="design-system/gallery", viewport="gallery", route="/gallery", settle=2.5),
]

SETS = {"current": CURRENT, "v2": V2, "design": DESIGN}
