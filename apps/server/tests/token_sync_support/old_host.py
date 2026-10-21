"""A Runtime Host from before token sync, for token_sync_e2e.py.

It enrolls and authenticates like storm-runtime (an Ed25519 key, signed with
the `openssl` CLI, since the standard library has no Ed25519), holds the host
link open, and says `hello` **without** `provider_credentials`, as every
runtime before 124p does. It records each command the server sends it and
never starts anything.
"""
import base64
import http.client
import json
import os
import shutil
import subprocess
import tempfile
import threading
import urllib.request

PREFIX = "storm-host-auth:v1:"


def b64url(raw):
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


class OldHost:
    def __init__(self, enrollment, name, workspaces):
        rest = enrollment.strip()[len("storm-enroll:v1:"):]
        self.base, self.server_id, _, token = rest.rsplit(":", 3)
        self.commands = []
        self.dir = tempfile.mkdtemp(prefix="storm-old-host-")
        self.key = os.path.join(self.dir, "key.pem")
        subprocess.run(["openssl", "genpkey", "-algorithm", "ed25519", "-out", self.key], check=True,
                       capture_output=True)
        der = subprocess.run(["openssl", "pkey", "-in", self.key, "-pubout", "-outform", "DER"],
                             check=True, capture_output=True).stdout
        tid = "hen_" + token[len("sen_"):].split(".")[0]
        enrolled = self._post("/v1/runtime/enroll", {
            "token": token, "public_key": b64url(der[-32:]), "key_id": "key_oldhost", "name": name,
            "signature": self._sign(f"{PREFIX}{self.server_id}:{tid}")})
        self.host_id = enrolled["host_id"]
        nonce = self._post("/v1/runtime/auth/challenge", {"host_id": self.host_id})["nonce"]
        issued = self._post("/v1/runtime/auth", {
            "host_id": self.host_id, "nonce": nonce,
            "signature": self._sign(f"{PREFIX}{self.server_id}:{self.host_id}:{nonce}")})
        self.bearer = f"Bearer {issued['token']}"
        self.linked = threading.Event()
        threading.Thread(target=self._link, daemon=True).start()
        self.linked.wait(10)
        self._post("/v1/runtime/hello", {
            "version": "0.9.0",
            "providers": [{"id": "claude-code", "kind": "cli", "interactions": ["terminal"],
                           "available": True}],
            "workspaces": workspaces, "max_sessions": 4}, auth=self.bearer)

    def _sign(self, message):
        msg = os.path.join(self.dir, "msg")
        with open(msg, "w") as f:
            f.write(message)
        sig = subprocess.run(["openssl", "pkeyutl", "-sign", "-inkey", self.key, "-rawin", "-in", msg],
                             check=True, capture_output=True).stdout
        return b64url(sig)

    def _post(self, path, body, auth=None):
        req = urllib.request.Request(f"{self.base}{path}", data=json.dumps(body).encode(), method="POST")
        req.add_header("content-type", "application/json")
        if auth:
            req.add_header("authorization", auth)
        with urllib.request.urlopen(req, timeout=10) as r:
            raw = r.read()
            return json.loads(raw) if raw else None

    def _link(self):
        req = urllib.request.Request(f"{self.base}/v1/runtime/link")
        req.add_header("authorization", self.bearer)
        req.add_header("accept", "text/event-stream")
        try:
            with urllib.request.urlopen(req, timeout=None) as r:
                self.linked.set()
                for line in r:
                    line = line.decode().rstrip("\r\n")
                    if line.startswith("data:"):
                        self.commands.append(json.loads(line[5:].strip()))
        except (OSError, ValueError, http.client.HTTPException):
            pass

    def starts(self):
        return [c for c in self.commands if c.get("type") == "start"]

    def close(self):
        shutil.rmtree(self.dir, ignore_errors=True)
