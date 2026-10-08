"""Real agent state for the Agents shots (slice 6).

Two `storm-runtime` hosts enrolled against the harness's server, exactly as
`apps/server/tests/gateway_e2e.py` enrolls one: `build-vm`, which stays
online, and `mac-mini`, which is restarted under a live session (so it fails
with `host_restart`) and then stopped. Their `claude-code` and `opencode`
providers run `agent.py` with the real MCP configuration, so every session,
name, context, write and Wrote row is the server's own state; the harness
only decides what the agent does next. Nothing in the client is faked.
"""

import json
import os
import signal
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "../../../../.."))
RUNTIME = os.path.join(ROOT, "apps/runtime/target/debug/storm-runtime")


def wait(fn, timeout, what):
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        try:
            last = fn()
            if last:
                return last
        except Exception as e:  # a transient read is retried
            last = e
        time.sleep(0.25)
    raise RuntimeError(f"timed out waiting for {what} (last: {str(last)[:300]})")


class Host:
    def __init__(self, world, name, workspaces):
        self.world = world
        self.name = name
        self.state = os.path.join(world.tmp, f"host-{name}")
        self.roots = os.path.join(world.tmp, f"workspaces-{name}")
        for ws in workspaces:
            os.makedirs(os.path.join(self.roots, ws), exist_ok=True)
        self.config = os.path.join(world.tmp, f"runtime-{name}.toml")
        agent = os.path.join(HERE, "agent.py")
        with open(self.config, "w") as f:
            f.write(f"""workspace_roots = ["{self.roots}"]
forbidden_roots = ["{os.path.join(world.tmp, 'state')}", "{os.path.join(world.tmp, 'vaults')}"]
max_sessions = 8

[[providers]]
id = "claude-code"
command = "{sys.executable}"
args = ["{agent}"]

[[providers]]
id = "opencode"
command = "{sys.executable}"
args = ["{agent}"]

[[providers]]
id = "shell"
command = "/bin/sh"
args = ["-i"]
""")
        self.proc = None
        self.id = None

    def enroll(self):
        _, issued = self.world.call("POST", "/v1/agent/hosts/enrollments",
                                    {"server_url": self.world.base})
        done = subprocess.run([RUNTIME, "enroll", "--state", self.state, "--name", self.name],
                              input=(issued["enrollment"] + "\n").encode(), capture_output=True)
        if done.returncode != 0:
            raise RuntimeError(f"enroll {self.name}: {done.stderr.decode()}")
        self.id = json.load(open(os.path.join(self.state, "host.json")))["host_id"]

    def start(self):
        log = open(os.path.join(self.world.tmp, f"runtime-{self.name}.log"), "a")
        self.proc = subprocess.Popen(
            [RUNTIME, "serve", "--state", self.state, "--config", self.config],
            stdout=log, stderr=subprocess.STDOUT)
        self.world.procs.append(self.proc)
        wait(lambda: self.world.host(self.id).get("status") == "online", 30,
             f"{self.name} online")

    def stop(self, sig=signal.SIGTERM):
        self.proc.send_signal(sig)
        self.proc.wait(timeout=10)


class Agent:
    """One session's scripted agent, through its inbox."""

    def __init__(self, host, workspace, session_id):
        self.dir = os.path.join(host.roots, workspace)
        self.sid = session_id
        self.n = 0
        self.rid = 0

    def _drop(self, item):
        self.n += 1
        inbox = os.path.join(self.dir, f"inbox-{self.sid}")
        wait(lambda: os.path.isdir(inbox), 20, "the agent's inbox")
        tmp = os.path.join(inbox, f"{self.n:05d}.tmp")
        with open(tmp, "w") as f:
            json.dump(item, f)
        os.rename(tmp, os.path.join(inbox, f"{self.n:05d}.json"))

    def say(self, text):
        self._drop({"say": text})
        time.sleep(0.2)

    def exit(self, code=0):
        self._drop({"exit": code})

    def _answer(self, rid):
        path = os.path.join(self.dir, f"transcript-{self.sid}-storm.jsonl")

        def found():
            if not os.path.exists(path):
                return None
            for line in open(path):
                m = json.loads(line) if line.strip() else {}
                if m.get("id") == rid and ("result" in m or "error" in m):
                    return m
            return None
        return wait(found, 30, f"answer {rid}")

    def request(self, method, params=None):
        self.rid += 1
        self._drop({"slug": "storm", "message": {"jsonrpc": "2.0", "id": self.rid,
                                                 "method": method, "params": params or {}}})
        answer = self._answer(self.rid)
        if "error" in answer:
            raise RuntimeError(f"{method}: {answer['error']}")
        return answer["result"]

    def initialize(self):
        self.request("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                    "clientInfo": {"name": "harness-agent", "version": "1"}})
        self._drop({"slug": "storm", "message": {"jsonrpc": "2.0",
                                                 "method": "notifications/initialized"}})

    def tool(self, name, args=None):
        result = self.request("tools/call", {"name": name, "arguments": args or {}})
        if result.get("isError"):
            raise RuntimeError(f"{name}: {result}")
        return result.get("structuredContent", {})


class AgentWorld:
    def __init__(self, harness):
        self.h = harness
        self.tmp = harness.tmp
        self.base = harness.base
        self.procs = harness.procs
        self.hosts = {}

    def call(self, method, path, body=None):
        return self.h.api(method, path, body)

    def host(self, host_id):
        _, hosts = self.call("GET", "/v1/agent/hosts")
        return next((h for h in hosts if h["id"] == host_id), {})

    def session(self, sid):
        return self.call("GET", f"/v1/agent/sessions/{sid}")[1]

    # ---- steps --------------------------------------------------------

    def enroll_hosts(self):
        """Hosts online, no sessions: the first-session state."""
        build = Host(self, "build-vm", ["storm", "site"])
        mac = Host(self, "mac-mini", ["storm"])
        for host in (build, mac):
            host.enroll()
            host.start()
            self.hosts[host.name] = host
        self.call("PUT", "/v1/config/mcp", {"agent_writes": True})
        ids = self.h.ids
        for key in ("note:kit/agents/Storm Lead.md", "note:work/Sprint notes.md",
                    "note:personal/projects/storm/Gateway spec.md"):
            vault = ids[f"vault:{key[5:].split('/')[0]}"]
            if key in ids:
                self.call("POST", f"/v1/vaults/{vault}/notes/{ids[key]}/opened")
                time.sleep(1.1)

    def launch(self, host, workspace, provider, name_key, context=None, write_vault=None):
        body = {"host_id": host.id, "workspace": workspace, "provider": provider,
                "interaction": "terminal", "terminal": {"cols": 80, "rows": 24}}
        if context:
            body["context"] = {"vault_id": self.h.ids["vault:personal"],
                               "note_id": self.h.ids[context]}
        if write_vault:
            body["write_vault_id"] = self.h.ids[f"vault:{write_vault}"]
        status, rec = self.call("POST", "/v1/agent/sessions", body)
        if status != 200:
            raise RuntimeError(f"launch: {status} {rec}")
        wait(lambda: self.session(rec["id"])["status"] == "running", 30, f"{name_key} running")
        self.h.ids[f"session:{name_key}"] = rec["id"]
        return rec, Agent(host, workspace, rec["id"])

    def note(self, key):
        vault = self.h.ids["vault:personal"]
        _, n = self.call("GET", f"/v1/vaults/{vault}/notes/{self.h.ids[key]}")
        return vault, n

    def run_sessions(self):
        """One live, two ended; the fourth is the core loop's, launched from
        the UI (`run_loop`)."""
        build, mac = self.hosts["build-vm"], self.hosts["mac-mini"]
        board = "note:personal/projects/storm/BOARD.md"

        # lint-fix: on mac-mini, which restarts under it and is then stopped.
        rec, a = self.launch(mac, "storm", "claude-code", "lint-fix")
        a.say("› running clippy…")
        mac.stop(signal.SIGKILL)
        mac.start()
        wait(lambda: self.session(rec["id"])["status"] == "failed", 30, "lint-fix failed")
        mac.stop()
        wait(lambda: self.host(mac.id).get("status") == "offline", 30, "mac-mini offline")

        # test-sweep: from BOARD, writes it, and finishes.
        rec, a = self.launch(build, "storm", "claude-code", "test-sweep",
                             context=board, write_vault="personal")
        a.initialize()
        a.tool("session_context")
        a.say("› read personal/projects/storm/BOARD")
        vault, n = self.note(board)
        a.tool("update_note", {"vault": vault, "note_id": n["id"], "base_version": n["version"],
                               "content": n["content"].replace(
                                   "- Relay settings screen",
                                   "- Relay settings screen\n- Flaky sync test")})
        a.say("› updated BOARD\n✓ done")
        time.sleep(0.5)
        a.exit(0)
        wait(lambda: self.session(rec["id"])["status"] == "completed", 30, "test-sweep completed")

        # docs-pass: OpenCode in another workspace, no note.
        _, a = self.launch(build, "site", "opencode", "docs-pass")
        a.say("› waiting for input")

    def run_loop(self):
        """The core loop's agent (handoff §11): the session the UI launched
        from Gateway spec reads its note, edits BOARD and creates a log note,
        all through the gateway, in its write vault."""
        board = "note:personal/projects/storm/BOARD.md"
        spec = self.h.ids["note:personal/projects/storm/Gateway spec.md"]

        def launched():
            _, sessions = self.call("GET", "/v1/agent/sessions")
            return next((s for s in sessions if (s.get("context") or {}).get("note_id") == spec
                         and s["status"] == "running"), None)
        rec = wait(launched, 30, "the session launched from Gateway spec")
        if rec["write_vault_id"] != self.h.ids["vault:personal"]:
            raise RuntimeError(f"the launcher's write vault: {rec}")
        self.h.ids["session:gateway-spec"] = rec["id"]
        host = next(h for h in self.hosts.values() if h.id == rec["host_id"])
        a = Agent(host, rec["workspace"], rec["id"])
        a.initialize()
        context = a.tool("session_context")
        if "Storm holds third-party MCP credentials" not in context.get("content", ""):
            raise RuntimeError(f"session_context: {context}")
        a.say("› read personal/projects/storm/Gateway spec")
        vault, n = self.note(board)
        a.tool("update_note", {"vault": vault, "note_id": n["id"], "base_version": n["version"],
                               "content": n["content"].replace(
                                   "## Next", "## Done\n\n- Gateway acceptance on Android\n\n## Next")})
        a.say("› updated BOARD: 1 item moved to Done")
        made = a.tool("create_note", {"vault": vault, "path": "projects/storm/log/2026-10-08.md",
                                      "content": "# 2026-10-08\n\n- Moved gateway acceptance to Done.\n"})
        self.h.ids["note:personal/projects/storm/log/2026-10-08.md"] = made["note"]["id"]
        a.say("› created personal/projects/storm/log/2026-10-08\n› waiting for input")
