"""Which screens the harness captures, per set.

Routes use `{vault:<name>}` and `{note:<vault>/<path>}` placeholders, resolved
against the fixture server at run time (ids are UUIDs minted at boot).

- `current`: the app as it is before Storm v2, for the record (`baseline/`).
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

V2 = []

SETS = {"current": CURRENT, "v2": V2}
