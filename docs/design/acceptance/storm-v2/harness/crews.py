"""Real crew state for the `crews` set (decision 138, slice 138-6).

Seeds the Round 4 scene through the server's own API, on the two hosts
`agents.py` enrolls: sessions launched with a role and a crew, crews made
around sessions, a label changed after launch, and `mac-mini` killed under a
live member so End all running has a host it can't reach. Every check of
something the server can see is made here against the real server and
printed as it runs; the UI-side checks are actions in `shots.py`. Nothing in
the client is faked.
"""

import signal
import time

import storm_auth
from agents import wait

LONG = "Install guide and relay docs refresh, Q4"  # 40 characters


class Checks:
    def __init__(self):
        self.results = []

    def __call__(self, name, ok, detail=""):
        self.results.append((name, bool(ok)))
        print(f"[crews] {'PASS' if ok else 'FAIL'} {name}" + ("" if ok else f" — {detail}"),
              flush=True)

    def failed(self):
        return [n for n, ok in self.results if not ok]


class CrewWorld:
    def __init__(self, harness, world):
        self.h = harness
        self.w = world
        self.check = Checks()
        harness.crew_checks = self.check

    def call(self, method, path, body=None):
        return self.w.call(method, path, body)

    def role(self, title):
        return {"vault_id": self.h.ids["vault:kit"],
                "note_id": self.h.ids[f"note:kit/agents/{title}.md"]}

    def crews(self):
        _, crews = self.call("GET", "/v1/agent/crews")
        return sorted(crews, key=lambda c: c["order"])

    def name(self, key, sid, title):
        """The name every surface shows: the agent's own title, once the host
        has reported it."""
        wait(lambda: (self.w.session(sid).get("title") or "") == title, 20, f"{key}'s title")
        self.h.ids[f"name:{key}"] = title

    def seed(self):
        w, ids, check = self.w, self.h.ids, self.check
        build, mac = w.hosts["build-vm"], w.hosts["mac-mini"]
        gw = "note:personal/projects/storm/Gateway spec.md"

        status, roles = self.call("GET", "/v1/agent/roles")
        check("Kit's agents/ notes are the roles, with their summaries (138e)",
              status == 200 and {r["title"] for r in roles} >= {
                  "Storm Coder", "Storm Reviewer", "Storm Lead"}, roles)

        # The ended ones first: a host runs at most 8 at once.
        rec, _ = w.launch(build, "storm", "shell", "kit-sweep")
        status, made = self.call("POST", "/v1/agent/crews",
                                 {"name": "Kit cleanup", "color": "peach",
                                  "session_ids": [rec["id"]]})
        check("a crew is made around a session", status == 201, (status, made))
        ids["crew:Kit cleanup"] = made["crew"]["id"]
        self.call("POST", f"/v1/agent/sessions/{rec['id']}/end")
        wait(lambda: w.session(rec["id"])["status"] == "stopped", 30, "kit-sweep stopped")
        ids["name:kit-sweep"] = w.session(rec["id"])["name"]

        rec, a = w.launch(build, "storm", "claude-code", "old-probe")
        a.title("✳ old-probe")
        self.name("old-probe", rec["id"], "old-probe")
        a.exit(0)
        wait(lambda: w.session(rec["id"])["status"] == "completed", 30, "old-probe completed")

        # Gateway release: made at launch from its context note, with a role.
        rec, a = w.launch(build, "storm", "claude-code", "gateway-impl", context=gw,
                          write_vault="personal",
                          extra={"role": self.role("Storm Coder"),
                                 "crew": {"new_name": "Gateway release"}})
        coder = self.role("Storm Coder")
        check("a launch with a role records it as the launch role and the first label (138f)",
              rec["launch_role"]["note_id"] == coder["note_id"]
              and rec["role"]["note_id"] == coder["note_id"], rec)
        check("a launch with a new crew makes it, the session its first member",
              rec["crew"]["order"] == 0, rec.get("crew"))
        gr = ids["crew:Gateway release"] = rec["crew"]["id"]
        a.initialize()
        a.title("◐ gateway-impl")
        got = a.tool("session_context")
        check("the agent reads its context and its role note through session_context",
              (got.get("context") or {}).get("title") == "Gateway spec"
              and (got.get("role") or {}).get("title") == "Storm Coder", got)
        self.name("gateway-impl", rec["id"], "gateway-impl")
        impl_agent = a

        rec, a = w.launch(build, "storm", "claude-code", "review-gateway", context=gw,
                          extra={"role": self.role("Storm Reviewer"), "crew": {"id": gr}})
        rev = rec["id"]
        a.initialize()
        a.title("✳ review-gateway")
        self.name("review-gateway", rev, "review-gateway")
        status, _ = self.call("PUT", f"/v1/agent/sessions/{rev}/role", self.role("Storm Lead"))
        view = w.session(rev)
        got = a.tool("session_context")
        check("a later label change moves only the label (138f)",
              status in (200, 204) and view["role"]["title"] == "Storm Lead"
              and view["launch_role"]["title"] == "Storm Reviewer"
              and got["role"]["title"] == "Storm Reviewer", (status, view, got))

        rec, a = w.launch(build, "site", "claude-code", "release-notes", extra={"crew": {"id": gr}})
        a.title("✳ release-notes")
        self.name("release-notes", rec["id"], "release-notes")
        a.exit(0)
        wait(lambda: w.session(rec["id"])["status"] == "completed", 30, "release-notes completed")

        rec, a = w.launch(mac, "storm", "claude-code", "relay-probe", extra={"crew": {"id": gr}})
        a.title("◐ relay-probe")
        self.name("relay-probe", rec["id"], "relay-probe")
        probe = rec["id"]

        rec, a = w.launch(build, "storm", "claude-code", "relay-timeouts")
        a.title("◐ relay-timeouts")
        self.name("relay-timeouts", rec["id"], "relay-timeouts")
        status, made = self.call("POST", "/v1/agent/crews", {
            "name": "Relay hardening", "color": "clay", "session_ids": [rec["id"]]})
        ids["crew:Relay hardening"] = made["crew"]["id"]
        timeouts = rec["id"]

        rec, a = w.launch(build, "site", "claude-code", "docs-pass")
        a.title("✳ docs-pass")
        self.name("docs-pass", rec["id"], "docs-pass")
        status, made = self.call("POST", "/v1/agent/crews", {
            "name": LONG, "color": "sky", "session_ids": [rec["id"]]})
        check("a 40-character name is accepted", status == 201, (status, made))
        ids["crew:long"] = made["crew"]["id"]

        rec, a = w.launch(build, "storm", "claude-code", "spike-sync")
        a.title("◐ spike-sync")
        self.name("spike-sync", rec["id"], "spike-sync")
        status, _ = self.call("PUT", f"/v1/agent/sessions/{rec['id']}/role",
                              self.role("Storm Researcher"))
        check("a session in no crew can carry a role (138d)",
              status in (200, 204) and w.session(rec["id"])["role"]["title"] == "Storm Researcher"
              and w.session(rec["id"])["crew"] is None, w.session(rec["id"]))

        rec, _ = w.launch(build, "storm", "shell", "shell-1",
                          extra={"role": self.role("Storm Lead")})
        check("a shell launched with a role gets the label only (138g)",
              rec["launch_role"] is None and rec["role"]["title"] == "Storm Lead", rec)
        self.call("DELETE", f"/v1/agent/sessions/{rec['id']}/role")
        ids["name:shell-1"] = rec["name"]

        # A session is never in two crews: moving it takes it out of the
        # first, and putting it back in front of its old neighbour restores
        # the order.
        impl = ids["session:gateway-impl"]
        self.call("PUT", f"/v1/agent/sessions/{impl}/crew",
                  {"crew_id": ids["crew:Relay hardening"]})
        _, sessions = self.call("GET", "/v1/agent/sessions")
        homes = [s["crew"]["id"] for s in sessions if s["id"] == impl and s["crew"]]
        check("a session is in at most one crew (138b)",
              homes == [ids["crew:Relay hardening"]], homes)
        self.call("PUT", f"/v1/agent/sessions/{impl}/crew", {"crew_id": gr, "before": rev})
        members = sorted((s for s in self.call("GET", "/v1/agent/sessions")[1]
                          if (s.get("crew") or {}).get("id") == gr),
                         key=lambda s: s["crew"]["order"])
        check("a move with `before` puts it back in place",
              [s["id"] for s in members][:2] == [impl, rev], [s["id"] for s in members])

        key = storm_auth.mint_mcp_key(self.h.base, self.h.session, name="crews acceptance")
        status, _ = storm_auth._call(self.h.base, "GET", "/v1/agent/crews", auth=key)
        check("an MCP key cannot reach the crew routes (138i)", status in (401, 403), status)

        check("crews are listed in the owner's order",
              [c["name"] for c in self.crews()] == [
                  "Kit cleanup", "Gateway release", "Relay hardening", LONG],
              [c["name"] for c in self.crews()])

        # relay-probe's host goes away under it: End all running must skip it.
        mac.stop(signal.SIGKILL)
        wait(lambda: w.host(mac.id).get("status") == "offline", 30, "mac-mini offline")
        self.probe, self.timeouts, self.impl_agent = probe, timeouts, impl_agent
        time.sleep(1)

    # ---- after UI actions ---------------------------------------------

    def after_undo(self):
        crews = self.crews()
        names = [c["name"] for c in crews]
        back = next((c for c in crews if c["name"] == "Kit cleanup"), None)
        self.check("Undo brought Kit cleanup back with its colour and place (138c)",
                   back is not None and back["color"] == "peach" and names[0] == "Kit cleanup"
                   and back["id"] != self.h.ids["crew:Kit cleanup"], names)

    def after_sync_in(self):
        self.check("a crew made on another device appeared without a reload, within one "
                   "poll (138j)", self.h.last_wait is not None and self.h.last_wait <= 16.5,
                   self.h.last_wait)

    def after_refused(self):
        names = [c["name"] for c in self.crews()]
        self.check("a change the server refused said why and changed nothing (138j)",
                   self.h.last_wait is not None
                   and names == ["Kit cleanup", "Gateway release", "Relay hardening"], names)

    def after_end_all(self):
        gr = self.h.ids["crew:Gateway release"]
        _, sessions = self.call("GET", "/v1/agent/sessions")
        members = {s["id"]: s for s in sessions if (s.get("crew") or {}).get("id") == gr}
        reachable = [s for i, s in members.items() if i != self.probe]
        self.check("End all running stopped every reachable member and skipped the "
                   "unreachable host (138k)",
                   all(s["status"] in ("stopped", "completed") for s in reachable)
                   and members[self.probe]["status"] not in ("stopped", "completed"),
                   {s["name"]: s["status"] for s in members.values()})
