"""Real session + crew state for the `public` set (Screenshot refresh spec,
2026-11-25). Built on agents.py's Host/Agent/AgentWorld against the demo
fixture (`fixture-demo/vaults`) — independent of the paths `current`/`v2`/
`crews` seed against their own fixture, so this never touches their default
behaviour. Nothing in the client is faked: writes go through the gateway.

Every session is labelled Claude Code, so the stand-in prints nothing to its
terminal (`silent=True`, spec §3): only `a.title(...)` runs, which is how a
session gets its name, never `a.say(...)`.
"""

import datetime

import agents

DAYBREAK = "projects/daybreak"


class PublicWorld:
    def __init__(self, harness, world):
        self.h = harness
        self.w = world

    def call(self, method, path, body=None):
        return self.w.call(method, path, body)

    def seed(self):
        """Two hosts; a 3-member crew plus a loose running and a loose ended
        session; the hero session that reads Launch plan and writes BOARD
        and a log note (the public set's one real agent edit). `daybreak` on
        build-vm is left empty, for the launcher shot's Where."""
        w, ids = self.w, self.h.ids
        build = agents.Host(w, "build-vm", ["personal", "docs", "daybreak"], silent=True)
        mac = agents.Host(w, "mac-mini", ["personal"], silent=True)
        for host in (build, mac):
            host.enroll()
            host.start()
            w.hosts[host.name] = host
        self.call("PUT", "/v1/config/mcp", {"agent_writes": True})

        _, a = w.launch(build, "personal", "claude-code", "writer-1")
        a.title("◐ Draft the release notes")
        _, a = w.launch(build, "docs", "claude-code", "reviewer-1")
        a.title("◐ Review the onboarding copy")
        _, a = w.launch(mac, "personal", "claude-code", "fixer-1")
        a.title("◐ Chase the export freeze")

        status, made = self.call("POST", "/v1/agent/crews", {
            "name": "Site launch", "color": "mint",
            "session_ids": [ids["session:writer-1"], ids["session:reviewer-1"],
                            ids["session:fixer-1"]]})
        if status != 201:
            raise RuntimeError(f"crew seed: {status} {made}")
        ids["crew:Site launch"] = made["crew"]["id"]

        rec, a = w.launch(build, "personal", "claude-code", "cleanup-1")
        a.title("✳ Clear the test fixtures")
        a.exit(0)
        agents.wait(lambda: w.session(rec["id"])["status"] == "completed", 30,
                    "cleanup-1 completed")

        # Named launch-plan: the server names a session from its context note
        # (Q12), and this one's is Launch plan.md.
        plan = f"note:personal/{DAYBREAK}/Launch plan.md"
        rec, a = w.launch(build, "personal", "claude-code", "launch-plan",
                          context=plan, write_vault="personal")
        a.initialize()
        a.title("◐ Set up crash reporting")
        context = a.tool("session_context")
        if "habit tracker" not in (context.get("context") or {}).get("content", ""):
            raise RuntimeError(f"session_context: {context}")
        vault, n = w.note(f"note:personal/{DAYBREAK}/BOARD.md")
        a.tool("update_note", {"vault": vault, "note_id": n["id"],
                               "base_version": n["version"],
                               "content": n["content"].replace(
                                   "## Next",
                                   "## Done\n\n- [x] Set up crash reporting\n\n## Next")})
        today = datetime.date.today().isoformat()
        made = a.tool("create_note", {"vault": vault,
                                      "path": f"{DAYBREAK}/log/{today}.md",
                                      "content": f"# {today}\n\n- Set up crash reporting.\n"})
        ids[f"note:personal/{DAYBREAK}/log/{today}.md"] = made["note"]["id"]
        a.title("✳ Set up crash reporting")
