"""Tests for the feedback sync service. Fake HTTP servers stand in for the Storm API and the intake Worker, shaped on the real routes rather than a full implementation of either."""

import hashlib
import http.server
import importlib.util
import json
import os
import re
import socketserver
import stat
import sys
import threading
import unittest
import urllib.parse
import uuid
from io import StringIO

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(os.path.dirname(HERE))
SCRIPT_PATH = os.path.join(HERE, "feedback-sync.py")
VECTORS_PATH = os.path.join(REPO_ROOT, "docs", "srp-vectors.json")


# The script's filename has a hyphen, so it can't be `import`ed by name.
def _load_module():
    spec = importlib.util.spec_from_file_location("feedback_sync", SCRIPT_PATH)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


fs = _load_module()


# A test-only Ed25519 signer, built on the module's own verified-correct
# primitives. The script only ever verifies; this exists so the fake Storm
# server can produce real signatures for `perform_pairing` to check.


def _test_ed25519_sign(message, seed):
    h = hashlib.sha512(seed).digest()
    a = int.from_bytes(h[:32], "little")
    a &= ~7
    a &= (1 << 255) - 1
    a |= 1 << 254
    big_a = fs._ed_scalarmult(fs._ED_B, a)
    pk = fs._ed_encodepoint(big_a)
    prefix = h[32:64]
    r = int.from_bytes(hashlib.sha512(prefix + message).digest(), "little") % fs._ED_L
    big_r = fs._ed_scalarmult(fs._ED_B, r)
    r_enc = fs._ed_encodepoint(big_r)
    s = (r + int.from_bytes(hashlib.sha512(r_enc + pk + message).digest(), "little") * a) % fs._ED_L
    return r_enc + fs._ed_encodeint(s), pk


def _verify_vector(case):
    # A decode failure is a refusal here too, same as a bad signature.
    try:
        pk = fs.b64url_decode(case["public_key_b64"])
        sig = fs.b64url_decode(case["signature_b64"])
    except ValueError:
        return False
    message = f"storm-challenge:v1:{case['server_id']}:{case['nonce']}".encode()
    return fs.ed25519_verify(sig, message, pk)


class Ed25519VerifyTests(unittest.TestCase):
    def test_against_the_repos_own_cross_implementation_vectors(self):
        # Signed with an independent implementation from this one, which is
        # the whole point.
        with open(VECTORS_PATH) as f:
            doc = json.load(f)
        cases = doc["verify_client_challenge"]
        for case in cases:
            got = _verify_vector(case)
            self.assertEqual(got, case["verifies"], case["name"])

    def test_sign_then_verify_round_trip(self):
        seed = os.urandom(32)
        message = b"storm-challenge:v1:srv_test:some-nonce-value-1234"
        sig, pk = _test_ed25519_sign(message, seed)
        self.assertTrue(fs.ed25519_verify(sig, message, pk))

    def test_tampered_signature_fails(self):
        seed = os.urandom(32)
        message = b"storm-challenge:v1:srv_test:some-nonce-value-1234"
        sig, pk = _test_ed25519_sign(message, seed)
        bad = bytearray(sig)
        bad[0] ^= 1
        self.assertFalse(fs.ed25519_verify(bytes(bad), message, pk))

    def test_tampered_message_fails(self):
        seed = os.urandom(32)
        message = b"storm-challenge:v1:srv_test:some-nonce-value-1234"
        sig, pk = _test_ed25519_sign(message, seed)
        self.assertFalse(fs.ed25519_verify(sig, message + b"x", pk))

    def test_malformed_inputs_return_false_not_raise(self):
        self.assertFalse(fs.ed25519_verify(b"short", b"m", b"short"))
        self.assertFalse(fs.ed25519_verify(b"\x00" * 64, b"m", b"\x00" * 32))

    def test_b64url_round_trip(self):
        raw = os.urandom(37)
        self.assertEqual(fs.b64url_decode(fs.b64url_encode(raw)), raw)

    def test_b64url_decode_rejects_the_standard_alphabet_and_padding(self):
        with self.assertRaises(ValueError):
            fs.b64url_decode("A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg")
        with self.assertRaises(ValueError):
            fs.b64url_decode("A6EHv_POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=")


# ---------------------------------------------------------------------------
# Templating
# ---------------------------------------------------------------------------


class TemplatingTests(unittest.TestCase):
    def test_first_line_truncates(self):
        self.assertEqual(fs.first_line("hello\nworld"), "hello")
        self.assertEqual(fs.first_line("x" * 200, limit=80), "x" * 80)
        self.assertEqual(fs.first_line(""), "")

    def test_short_title_adds_ellipsis_only_when_truncated(self):
        self.assertEqual(fs.short_title("short"), "short")
        long = "y" * 100
        got = fs.short_title(long, limit=60)
        self.assertEqual(len(got), 60)
        self.assertTrue(got.endswith("…"))

    def test_fenced_uses_longer_fence_when_text_has_backticks(self):
        text = "here is a ```fenced block``` inside"
        out = fs.fenced(text, "text")
        # The fence must not appear inside the body, or it would close early.
        lines = out.split("\n")
        fence = lines[0][: len(lines[0]) - len("text")]
        self.assertGreater(len(fence), 3)
        body = "\n".join(lines[1:-1])
        self.assertNotIn(fence, body)
        self.assertEqual(lines[-1], fence)

    def test_fenced_default_is_three_backticks(self):
        out = fs.fenced("plain text, no backticks", "text")
        self.assertTrue(out.startswith("```text\n"))
        self.assertTrue(out.endswith("```"))

    def test_render_context_is_defensive_about_shape(self):
        ctx = fs.render_context({"screen": "Settings", "last_actions": ["a", "b"], "error": None})
        self.assertIn("- Screen: Settings", ctx)
        self.assertIn("- Last actions: a, b", ctx)
        self.assertIn('Error (from "Report this"): —', ctx)

        # Garbage shapes must not raise.
        ctx2 = fs.render_context("not a dict")
        self.assertIn("—", ctx2)
        ctx3 = fs.render_context({"screen": {"nested": 1}, "last_actions": 42, "error": True})
        self.assertIsInstance(ctx3, str)

    def test_render_note_has_no_id_created_modified(self):
        report = {
            "id": "rep1",
            "kind": "bug",
            "text": "Line one\nmore detail",
            "email": "a@example.com",
            "client": {"version": "1.2.3", "platform": "linux-x86_64 · flutter-desktop"},
            "bundle": {"ok": True},
            "received_at": "2026-11-19T00:00:00Z",
            "has_screenshot": True,
        }
        content = fs.render_note("FB-0001", report)
        self.assertNotIn("\nid:", content)
        self.assertNotIn("\ncreated:", content)
        self.assertNotIn("\nmodified:", content)
        self.assertIn("ref: FB-0001", content)
        self.assertIn("intake: rep1", content)
        self.assertIn("kind: bug", content)
        self.assertIn("version: 1.2.3", content)
        self.assertIn("# FB-0001 — Line one", content)
        self.assertIn("Line one\nmore detail", content)
        self.assertIn("![[FB-0001.png]]", content)

    def test_render_note_without_screenshot(self):
        report = {
            "id": "rep1", "kind": "idea", "text": "An idea",
            "client": {}, "bundle": {}, "received_at": "t", "has_screenshot": False,
        }
        content = fs.render_note("FB-0002", report)
        self.assertIn("- Screenshot: none", content)
        self.assertNotIn("![[", content)

    def test_render_note_bundle_with_backticks_is_safely_fenced(self):
        report = {
            "id": "rep1", "kind": "other", "text": "t",
            "client": {}, "bundle": {"note": "```danger```"}, "received_at": "t",
            "has_screenshot": False,
        }
        content = fs.render_note("FB-0003", report)
        # The diagnostics section must still be a single well-formed fence.
        self.assertIn("## Diagnostics", content)
        diag = content.split("## Diagnostics\n", 1)[1]
        fence_line = diag.split("\n", 1)[0]
        self.assertGreater(len(fence_line), len("```text"))

    def test_feedback_md_line_shape(self):
        line = fs.feedback_md_line("FB-0001", "Storm/Feedback", "bug", "Something broke", "1.2.3")
        self.assertEqual(
            line,
            '- [ ] [[Storm/Feedback/Inbox/FB-0001|FB-0001]] · bug · "Something broke" · 1.2.3',
        )


class FeedbackMdUpdateTests(unittest.TestCase):
    def test_removes_placeholder_and_inserts(self):
        content = "# Feedback\n\n## Inbox (new)\n\n_None yet._\n\n## Triaged\n\nstuff\n"
        new = fs.update_feedback_md_content(
            content, fs.FEEDBACK_MD_INBOX_HEADING, "FB-0001", "- bullet FB-0001"
        )
        self.assertNotIn(fs.FEEDBACK_MD_PLACEHOLDER, new)
        self.assertIn("- bullet FB-0001", new)
        self.assertIn("## Triaged\n\nstuff", new)

    def test_appends_to_existing_list(self):
        content = "## Inbox (new)\n\n- old bullet FB-0000\n\n## Triaged\n"
        new = fs.update_feedback_md_content(
            content, fs.FEEDBACK_MD_INBOX_HEADING, "FB-0001", "- bullet FB-0001"
        )
        self.assertIn("- old bullet FB-0000", new)
        self.assertIn("- bullet FB-0001", new)
        # Order: old content, then the new bullet, before the next heading.
        self.assertLess(new.index("FB-0000"), new.index("FB-0001"))
        self.assertLess(new.index("- bullet FB-0001"), new.index("## Triaged"))

    def test_idempotent_when_ref_already_present(self):
        content = "## Inbox (new)\n\n- [ ] [[x|FB-0001]] · bug · \"t\" · 1\n\n## Triaged\n"
        new = fs.update_feedback_md_content(
            content, fs.FEEDBACK_MD_INBOX_HEADING, "FB-0001", "- a different bullet FB-0001"
        )
        self.assertEqual(new, content)

    def test_missing_heading_appends_a_new_section(self):
        content = "# Feedback\n\nsome preamble\n"
        new = fs.update_feedback_md_content(
            content, fs.FEEDBACK_MD_INBOX_HEADING, "FB-0001", "- bullet FB-0001"
        )
        self.assertIn(fs.FEEDBACK_MD_INBOX_HEADING, new)
        self.assertIn("- bullet FB-0001", new)


class FrontmatterScalarTests(unittest.TestCase):
    def test_reads_a_scalar(self):
        content = "---\nref: FB-0005\nintake: abc123\nstatus: new\n---\n\n# body\n"
        self.assertEqual(fs.parse_frontmatter_scalar(content, "ref"), "FB-0005")
        self.assertEqual(fs.parse_frontmatter_scalar(content, "intake"), "abc123")
        self.assertIsNone(fs.parse_frontmatter_scalar(content, "nope"))

    def test_no_frontmatter_returns_none(self):
        self.assertIsNone(fs.parse_frontmatter_scalar("# just a note\n", "ref"))

    def test_handles_bom(self):
        content = "﻿---\nref: FB-0006\n---\n"
        self.assertEqual(fs.parse_frontmatter_scalar(content, "ref"), "FB-0006")

    def test_empty_value_is_none(self):
        content = "---\nissue:\nref: FB-0007\n---\n"
        self.assertIsNone(fs.parse_frontmatter_scalar(content, "issue"))
        self.assertEqual(fs.parse_frontmatter_scalar(content, "ref"), "FB-0007")


class PairingCodeParsingTests(unittest.TestCase):
    def test_parses_a_full_uri(self):
        uri = "storm://pair?v=1&sid=srv_ABC&pk=PK123&n=nonceXYZ&exp=2026&addr=1.2.3.4:8484"
        parsed = fs.parse_pairing_code(uri)
        self.assertEqual(parsed["nonce"], "nonceXYZ")
        self.assertEqual(parsed["server_id"], "srv_ABC")
        self.assertEqual(parsed["public_key"], "PK123")

    def test_strips_embedded_whitespace(self):
        uri = "storm://pair?v=1&sid=srv_ABC&pk=PK123&n=no ncXYZ&exp=2026&addr=1.2.3.4:8484"
        parsed = fs.parse_pairing_code(uri)
        self.assertEqual(parsed["nonce"], "noncXYZ")

    def test_bare_nonce(self):
        parsed = fs.parse_pairing_code("  just-a-nonce-value  ")
        self.assertEqual(parsed["nonce"], "just-a-nonce-value")
        self.assertIsNone(parsed["server_id"])

    def test_empty_code_raises(self):
        with self.assertRaises(fs.PairingError):
            fs.parse_pairing_code("   ")

    def test_wrong_scheme_raises(self):
        with self.assertRaises(fs.PairingError):
            fs.parse_pairing_code("https://pair?n=x")

    def test_uri_missing_nonce_raises(self):
        with self.assertRaises(fs.PairingError):
            fs.parse_pairing_code("storm://pair?v=1&sid=srv_ABC")


# ---------------------------------------------------------------------------
# State / credentials on disk
# ---------------------------------------------------------------------------


class StateAndCredsTests(unittest.TestCase):
    def setUp(self):
        import tempfile

        self.tmp = tempfile.TemporaryDirectory()
        self.state_dir = os.path.join(self.tmp.name, "state")

    def tearDown(self):
        self.tmp.cleanup()

    def test_device_creds_round_trip_and_permissions(self):
        creds = fs.DeviceCreds(
            device_id="dev1", device_secret="s3cr3t", server_id="srv_x",
            access_token="at", access_expires="2027-01-01T00:00:00Z",
            refresh_token="rt", refresh_expires="2027-06-01T00:00:00Z",
        )
        creds.save(self.state_dir)
        path = fs.DeviceCreds._path(self.state_dir)
        mode = stat.S_IMODE(os.stat(path).st_mode)
        self.assertEqual(mode, 0o600)

        loaded = fs.DeviceCreds.load(self.state_dir)
        self.assertEqual(loaded.device_id, "dev1")
        self.assertEqual(loaded.device_secret, "s3cr3t")
        self.assertEqual(loaded.refresh_token, "rt")

    def test_device_creds_missing_is_none(self):
        self.assertIsNone(fs.DeviceCreds.load(self.state_dir))

    def test_state_round_trip(self):
        state = fs.State.load(self.state_dir)
        self.assertEqual(state.synced, {})
        self.assertIsNone(state.next_ref)
        state.synced["r1"] = "FB-0001"
        state.next_ref = 2
        state.save()

        reloaded = fs.State.load(self.state_dir)
        self.assertEqual(reloaded.synced, {"r1": "FB-0001"})
        self.assertEqual(reloaded.next_ref, 2)

    def test_load_config_requires_fields(self):
        with self.assertRaises(SystemExit):
            fs.load_config(env={})

    def test_load_config_defaults(self):
        cfg = fs.load_config(
            env={
                "STORM_URL": "http://host:8484/",
                "STORM_VAULT": "v1",
                "INTAKE_URL": "https://intake.example/",
                "SYNC_TOKEN": "tok",
            }
        )
        self.assertEqual(cfg.storm_url, "http://host:8484")
        self.assertEqual(cfg.feedback_folder, fs.DEFAULT_FEEDBACK_FOLDER)
        self.assertEqual(cfg.state_dir, fs.DEFAULT_STATE_DIR)


# ---------------------------------------------------------------------------
# Fake servers
# ---------------------------------------------------------------------------


class _ThreadingHTTPServer(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def _read_json_body(handler):
    length = int(handler.headers.get("Content-Length", 0) or 0)
    raw = handler.rfile.read(length) if length else b""
    if not raw:
        return {}
    return json.loads(raw.decode("utf-8"))


def _send_json(handler, status, body):
    data = json.dumps(body).encode("utf-8")
    handler.send_response(status)
    handler.send_header("Content-Type", "application/json")
    handler.send_header("Content-Length", str(len(data)))
    handler.end_headers()
    handler.wfile.write(data)


def _send_bytes(handler, status, data, content_type="application/octet-stream"):
    handler.send_response(status)
    handler.send_header("Content-Type", content_type)
    handler.send_header("Content-Length", str(len(data)))
    handler.end_headers()
    if data:
        handler.wfile.write(data)


def _send_empty(handler, status):
    handler.send_response(status)
    handler.send_header("Content-Length", "0")
    handler.end_headers()


class FakeVault:
    """An in-memory stand-in for one Storm vault's notes."""

    def __init__(self):
        self.notes = {}  # id -> {id, path, title, version, content}
        self.force_conflicts = 0

    def tree(self):
        return {
            "notes": [
                {
                    "id": n["id"],
                    "path": n["path"],
                    "title": n["title"],
                    "version": n["version"],
                    "content_hash": "x",
                    "created": "2026-01-01T00:00:00Z",
                    "modified": "2026-01-01T00:00:00Z",
                    "size": len(n["content"]),
                }
                for n in self.notes.values()
            ],
            "folders": [],
            "seq": 1,
        }

    def get_note(self, note_id):
        n = self.notes.get(note_id)
        if n is None:
            return None
        return {
            "id": n["id"],
            "path": n["path"],
            "title": n["title"],
            "version": n["version"],
            "content_hash": "x",
            "created": "2026-01-01T00:00:00Z",
            "modified": "2026-01-01T00:00:00Z",
            "size": len(n["content"]),
            "content": n["content"],
        }

    def _row(self, n):
        return {
            "id": n["id"], "path": n["path"], "title": n["title"], "version": n["version"],
            "content_hash": "x", "created": "2026-01-01T00:00:00Z",
            "modified": "2026-01-01T00:00:00Z", "size": len(n["content"]),
        }

    def seed_note(self, path, content):
        note_id = str(uuid.uuid4())
        self.notes[note_id] = {
            "id": note_id, "path": path,
            "title": path.rsplit("/", 1)[-1].removesuffix(".md"),
            "version": 1, "content": content,
        }
        return note_id

    def create_note(self, path, content):
        for n in self.notes.values():
            if n["path"] == path:
                return None  # "a note already exists at <path>"
        note_id = str(uuid.uuid4())
        n = {
            "id": note_id, "path": path,
            "title": path.rsplit("/", 1)[-1].removesuffix(".md"),
            "version": 1, "content": content,
        }
        self.notes[note_id] = n
        return {"note": self._row(n), "content": n["content"], "seq": 1, "merged": False, "conflict": False}

    def put_note(self, note_id, base_version, content):
        n = self.notes.get(note_id)
        if n is None:
            return None
        if self.force_conflicts > 0:
            self.force_conflicts -= 1
            n["version"] += 1
            return {
                "note": self._row(n), "content": n["content"], "seq": n["version"],
                "merged": False, "conflict": True,
            }
        n["version"] += 1
        n["content"] = content
        return {
            "note": self._row(n), "content": n["content"], "seq": n["version"],
            "merged": False, "conflict": False,
        }

    def put_attachment(self, path, data):
        return {"path": path, "size": len(data)}


class StormServerState:
    def __init__(self, vault):
        self.vault = vault
        self.attachments = {}
        self.server_id = "srv_TEST"
        self.key_id = "key_test"
        self.seed = os.urandom(32)
        _, pk = _test_ed25519_sign(b"", self.seed)
        self.public_key_b64 = fs.b64url_encode(pk)
        self.pairing_nonce = None
        self.device_id = None
        self.device_secret = None
        self.password = "the-operator-password"
        self.access_token = "access-" + uuid.uuid4().hex
        self.refresh_token = "refresh-" + uuid.uuid4().hex
        self.refresh_calls = 0

    def sign(self, message):
        sig, _ = _test_ed25519_sign(message, self.seed)
        return sig

    def server_info(self):
        return {
            "server_id": self.server_id,
            "name": "test",
            "key_id": self.key_id,
            "algorithm": "ed25519",
            "public_key": self.public_key_b64,
            "relays": [],
            "relays_sig": "",
        }

    def issue_session(self):
        self.access_token = "access-" + uuid.uuid4().hex
        self.refresh_token = "refresh-" + uuid.uuid4().hex
        return {
            "session_id": "ses1", "user_id": "u1", "device_id": self.device_id or "dev1",
            "access_token": self.access_token, "refresh_token": self.refresh_token,
            "expires": "2099-01-01T00:00:00Z", "refresh_expires": "2099-06-01T00:00:00Z",
        }


class StormHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def _state(self):
        return self.server.state

    def _authorized(self):
        return self.headers.get("Authorization") == f"Bearer {self._state().access_token}"

    def do_GET(self):
        state = self._state()
        path = urllib.parse.urlparse(self.path).path
        if path == "/v1/server":
            return _send_json(self, 200, state.server_info())
        m = re.match(r"^/v1/vaults/[^/]+/tree$", path)
        if m:
            if not self._authorized():
                return _send_json(self, 401, {"error": "unauthorized"})
            return _send_json(self, 200, state.vault.tree())
        m = re.match(r"^/v1/vaults/[^/]+/notes/([^/]+)$", path)
        if m:
            if not self._authorized():
                return _send_json(self, 401, {"error": "unauthorized"})
            note = state.vault.get_note(urllib.parse.unquote(m.group(1)))
            if note is None:
                return _send_json(self, 404, {"error": "not found"})
            return _send_json(self, 200, note)
        return _send_json(self, 404, {"error": "not found"})

    def do_POST(self):
        state = self._state()
        path = urllib.parse.urlparse(self.path).path
        if path == "/v1/server/challenge":
            body = _read_json_body(self)
            nonce = body.get("nonce", "")
            message = f"storm-challenge:v1:{state.server_id}:{nonce}".encode("utf-8")
            sig = state.sign(message)
            return _send_json(
                self, 200,
                {"server_id": state.server_id, "key_id": state.key_id, "signature": fs.b64url_encode(sig)},
            )
        if path == "/v1/pair":
            body = _read_json_body(self)
            if state.pairing_nonce is None or body.get("n") != state.pairing_nonce:
                return _send_json(self, 401, {"error": "invalid or missing token"})
            state.device_id = "dev-" + uuid.uuid4().hex[:8]
            state.device_secret = "sec-" + uuid.uuid4().hex
            return _send_json(
                self, 200,
                {
                    "device_id": state.device_id, "device_secret": state.device_secret,
                    "server_id": state.server_id, "public_key": state.public_key_b64,
                    "key_id": state.key_id,
                },
            )
        if path == "/v1/auth/login":
            device_header = self.headers.get("Authorization", "")
            expected = f"StormDevice {state.device_id}:{state.device_secret}"
            if device_header != expected:
                return _send_json(self, 401, {"error": "invalid or missing token"})
            body = _read_json_body(self)
            if body.get("password") != state.password:
                return _send_json(self, 401, {"error": "invalid_credentials"})
            return _send_json(self, 200, state.issue_session())
        if path == "/v1/auth/refresh":
            body = _read_json_body(self)
            state.refresh_calls += 1
            if body.get("refresh_token") != state.refresh_token:
                return _send_json(self, 401, {"error": "invalid refresh token"})
            return _send_json(self, 200, state.issue_session())
        m = re.match(r"^/v1/vaults/[^/]+/notes$", path)
        if m:
            if not self._authorized():
                return _send_json(self, 401, {"error": "unauthorized"})
            body = _read_json_body(self)
            result = state.vault.create_note(body["path"], body["content"])
            if result is None:
                return _send_json(self, 400, {"error": f"a note already exists at {body['path']}"})
            return _send_json(self, 200, result)
        return _send_json(self, 404, {"error": "not found"})

    def do_PUT(self):
        state = self._state()
        path = urllib.parse.urlparse(self.path).path
        if not self._authorized():
            return _send_json(self, 401, {"error": "unauthorized"})
        m = re.match(r"^/v1/vaults/[^/]+/notes/([^/]+)$", path)
        if m:
            body = _read_json_body(self)
            result = state.vault.put_note(urllib.parse.unquote(m.group(1)), body["base_version"], body["content"])
            if result is None:
                return _send_json(self, 404, {"error": "not found"})
            return _send_json(self, 200, result)
        m = re.match(r"^/v1/vaults/[^/]+/attachments/(.+)$", path)
        if m:
            length = int(self.headers.get("Content-Length", 0) or 0)
            data = self.rfile.read(length) if length else b""
            att_path = urllib.parse.unquote(m.group(1))
            state.attachments[att_path] = data
            return _send_json(self, 200, state.vault.put_attachment(att_path, data))
        return _send_json(self, 404, {"error": "not found"})


class WorkerState:
    def __init__(self):
        self.token = "sync-token-xyz"
        self.reports = {}  # id -> report dict
        self.screenshots = {}  # id -> bytes
        self.deleted = []

    def add_report(self, report):
        self.reports[report["id"]] = report


class WorkerHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def _authorized(self):
        return self.headers.get("Authorization") == f"Bearer {self.server.state.token}"

    def do_GET(self):
        state = self.server.state
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path == "/v1/reports":
            if not self._authorized():
                return _send_json(self, 401, {"error": "unauthorized"})
            limit = int(urllib.parse.parse_qs(parsed.query).get("limit", ["50"])[0])
            ids = sorted(state.reports.keys())[:limit]
            reports = []
            for rid in ids:
                r = dict(state.reports[rid])
                r["has_screenshot"] = rid in state.screenshots
                reports.append(r)
            return _send_json(self, 200, {"reports": reports})
        m = re.match(r"^/v1/reports/([^/]+)/screenshot$", parsed.path)
        if m:
            if not self._authorized():
                return _send_json(self, 401, {"error": "unauthorized"})
            rid = urllib.parse.unquote(m.group(1))
            png = state.screenshots.get(rid)
            if png is None:
                return _send_json(self, 404, {"error": "not found"})
            return _send_bytes(self, 200, png, "image/png")
        return _send_json(self, 404, {"error": "not found"})

    def do_DELETE(self):
        state = self.server.state
        m = re.match(r"^/v1/reports/([^/]+)$", urllib.parse.urlparse(self.path).path)
        if not m:
            return _send_json(self, 404, {"error": "not found"})
        if not self._authorized():
            return _send_json(self, 401, {"error": "unauthorized"})
        rid = urllib.parse.unquote(m.group(1))
        if rid not in state.reports:
            return _send_empty(self, 404)
        del state.reports[rid]
        state.screenshots.pop(rid, None)
        state.deleted.append(rid)
        return _send_empty(self, 204)


def _start_server(handler_cls, state):
    server = _ThreadingHTTPServer(("127.0.0.1", 0), handler_cls)
    server.state = state
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, thread


def _stop_server(server):
    server.shutdown()
    server.server_close()


def make_report(report_id, kind="bug", text="Something broke", has_screenshot=False, **extra):
    r = {
        "id": report_id,
        "schema": 1,
        "install_id": "11111111-1111-1111-1111-111111111111",
        "kind": kind,
        "text": text,
        "client": {"version": "1.0.0", "platform": "linux-x86_64 · flutter-desktop"},
        "bundle": {"server": {"version": "0.18.0"}},
        "received_at": "2026-11-19T00:00:00Z",
    }
    r.update(extra)
    return r


class FakeServersTestCase(unittest.TestCase):
    def setUp(self):
        import tempfile

        self.tmp = tempfile.TemporaryDirectory()
        self.state_dir = os.path.join(self.tmp.name, "state")

        self.vault = FakeVault()
        self.feedback_note_id = self.vault.seed_note(
            "Storm/Feedback/Feedback.md",
            "# Feedback\n\n## Inbox (new)\n\n_None yet._\n\n## Triaged\n",
        )
        self.storm_state = StormServerState(self.vault)
        self.storm_server, self.storm_thread = _start_server(StormHandler, self.storm_state)
        self.storm_url = f"http://127.0.0.1:{self.storm_server.server_address[1]}"

        self.worker_state = WorkerState()
        self.worker_server, self.worker_thread = _start_server(WorkerHandler, self.worker_state)
        self.worker_url = f"http://127.0.0.1:{self.worker_server.server_address[1]}"

        self.cfg = fs.Config(
            storm_url=self.storm_url,
            vault="vault1",
            feedback_folder="Storm/Feedback",
            intake_url=self.worker_url,
            sync_token=self.worker_state.token,
            state_dir=self.state_dir,
        )

        creds = fs.DeviceCreds(
            device_id="dev1", device_secret="sec1", server_id=self.storm_state.server_id,
            access_token=self.storm_state.access_token, access_expires="2099-01-01T00:00:00Z",
            refresh_token=self.storm_state.refresh_token, refresh_expires="2099-06-01T00:00:00Z",
        )
        creds.save(self.state_dir)

    def tearDown(self):
        _stop_server(self.storm_server)
        _stop_server(self.worker_server)
        self.tmp.cleanup()

    def _feedback_md_content(self):
        return self.vault.get_note(self.feedback_note_id)["content"]

    def _capture_run(self):
        buf = StringIO()
        old_stdout = sys.stdout
        sys.stdout = buf
        try:
            rc = fs.run(self.cfg)
        finally:
            sys.stdout = old_stdout
        return rc, buf.getvalue()


class PerformPairingTests(unittest.TestCase):
    def setUp(self):
        self.vault = FakeVault()
        self.storm_state = StormServerState(self.vault)
        self.storm_state.pairing_nonce = "the-pairing-nonce-123456"
        self.storm_server, self.thread = _start_server(StormHandler, self.storm_state)
        self.storm_url = f"http://127.0.0.1:{self.storm_server.server_address[1]}"

    def tearDown(self):
        _stop_server(self.storm_server)

    def test_happy_path_with_bare_nonce(self):
        creds = fs.perform_pairing(
            self.storm_url, self.storm_state.pairing_nonce, "feedback-sync", "linux",
            "162-6.test", self.storm_state.password,
        )
        self.assertEqual(creds.server_id, self.storm_state.server_id)
        self.assertTrue(creds.device_id)
        self.assertTrue(creds.access_token)
        self.assertTrue(creds.refresh_token)

    def test_happy_path_with_full_uri(self):
        uri = (
            f"storm://pair?v=1&sid={self.storm_state.server_id}"
            f"&pk={self.storm_state.public_key_b64}&n={self.storm_state.pairing_nonce}"
            f"&exp=2099&addr=127.0.0.1:1"
        )
        creds = fs.perform_pairing(
            self.storm_url, uri, "feedback-sync", "linux", "162-6.test", self.storm_state.password,
        )
        self.assertEqual(creds.device_id, self.storm_state.device_id)

    def test_wrong_password_raises(self):
        with self.assertRaises(fs.ApiError):
            fs.perform_pairing(
                self.storm_url, self.storm_state.pairing_nonce, "feedback-sync", "linux",
                "162-6.test", "not-the-password",
            )

    def test_mismatched_server_id_in_code_is_refused_before_touching_the_network_result(self):
        uri = (
            f"storm://pair?v=1&sid=srv_SOMETHING_ELSE"
            f"&pk={self.storm_state.public_key_b64}&n={self.storm_state.pairing_nonce}"
            f"&exp=2099&addr=127.0.0.1:1"
        )
        with self.assertRaises(fs.PairingError):
            fs.perform_pairing(
                self.storm_url, uri, "feedback-sync", "linux", "162-6.test", self.storm_state.password,
            )

    def test_tampered_challenge_signature_is_refused(self):
        # The server signs with a different key than the pairing code named.
        original_sign = self.storm_state.sign
        other_seed = os.urandom(32)

        def wrong_sign(message):
            sig, _ = _test_ed25519_sign(message, other_seed)
            return sig

        self.storm_state.sign = wrong_sign
        try:
            with self.assertRaises(fs.PairingError):
                fs.perform_pairing(
                    self.storm_url, self.storm_state.pairing_nonce, "feedback-sync", "linux",
                    "162-6.test", self.storm_state.password,
                )
        finally:
            self.storm_state.sign = original_sign


class RunHappyPathTests(FakeServersTestCase):
    def test_sync_one_report_without_screenshot(self):
        self.worker_state.add_report(make_report("rep-1", kind="bug", text="Export failed\nmore info"))
        rc, _ = self._capture_run()
        self.assertEqual(rc, 0)

        notes = {n["path"]: n for n in self.vault.tree()["notes"]}
        self.assertIn("Storm/Feedback/Inbox/FB-0001.md", notes)
        note = self.vault.get_note(notes["Storm/Feedback/Inbox/FB-0001.md"]["id"])
        self.assertIn("ref: FB-0001", note["content"])
        self.assertIn("intake: rep-1", note["content"])
        self.assertIn("Export failed", note["content"])

        self.assertIn("rep-1", self.worker_state.deleted)
        self.assertEqual(self.worker_state.reports, {})

        fb_content = self._feedback_md_content()
        self.assertIn("FB-0001", fb_content)
        self.assertNotIn(fs.FEEDBACK_MD_PLACEHOLDER, fb_content)

    def test_sync_one_report_with_screenshot(self):
        self.worker_state.add_report(make_report("rep-2", text="Crash on launch"))
        self.worker_state.screenshots["rep-2"] = b"\x89PNG\r\n\x1a\nFAKE"
        rc, _ = self._capture_run()
        self.assertEqual(rc, 0)

        notes = {n["path"]: n for n in self.vault.tree()["notes"]}
        note = self.vault.get_note(notes["Storm/Feedback/Inbox/FB-0001.md"]["id"])
        self.assertIn("![[FB-0001.png]]", note["content"])
        self.assertEqual(
            self.storm_state.attachments.get("Storm/Feedback/Inbox/FB-0001.png"),
            b"\x89PNG\r\n\x1a\nFAKE",
        )

    def test_numbering_continues_from_existing_notes(self):
        self.vault.seed_note(
            "Storm/Feedback/Inbox/FB-0005.md",
            "---\nref: FB-0005\nintake: some-older-report\nstatus: triaged\n---\n\n# old\n",
        )
        self.worker_state.add_report(make_report("rep-3", text="New one"))
        rc, _ = self._capture_run()
        self.assertEqual(rc, 0)

        notes = {n["path"] for n in self.vault.tree()["notes"]}
        self.assertIn("Storm/Feedback/Inbox/FB-0006.md", notes)
        self.assertNotIn("Storm/Feedback/Inbox/FB-0001.md", notes)

    def test_multiple_reports_get_sequential_numbers(self):
        self.worker_state.add_report(make_report("rep-a", text="first"))
        self.worker_state.add_report(make_report("rep-b", text="second"))
        rc, _ = self._capture_run()
        self.assertEqual(rc, 0)
        notes = {n["path"] for n in self.vault.tree()["notes"]}
        self.assertIn("Storm/Feedback/Inbox/FB-0001.md", notes)
        self.assertIn("Storm/Feedback/Inbox/FB-0002.md", notes)

    def test_feedback_md_insertion_with_an_existing_list(self):
        self.vault.notes[self.feedback_note_id]["content"] = (
            "# Feedback\n\n## Inbox (new)\n\n"
            '- [ ] [[Storm/Feedback/Inbox/FB-0000|FB-0000]] · bug · "older" · 1.0.0\n\n'
            "## Triaged\n"
        )
        self.worker_state.add_report(make_report("rep-4", text="newer one"))
        rc, _ = self._capture_run()
        self.assertEqual(rc, 0)
        content = self._feedback_md_content()
        self.assertIn("FB-0000", content)
        self.assertIn("FB-0001", content)

    def test_feedback_md_conflict_is_retried(self):
        self.vault.force_conflicts = 2
        self.worker_state.add_report(make_report("rep-5", text="conflict test"))
        rc, _ = self._capture_run()
        self.assertEqual(rc, 0)
        content = self._feedback_md_content()
        self.assertIn("FB-0001", content)

    def test_backticks_in_bundle_do_not_break_the_note(self):
        self.worker_state.add_report(
            make_report("rep-6", text="see output:\n```oops```", bundle={"log": "```nested```"})
        )
        rc, _ = self._capture_run()
        self.assertEqual(rc, 0)
        notes = {n["path"]: n for n in self.vault.tree()["notes"]}
        note = self.vault.get_note(notes["Storm/Feedback/Inbox/FB-0001.md"]["id"])
        self.assertIn("## Diagnostics", note["content"])

    def test_one_bad_report_does_not_stop_the_others(self):
        self.worker_state.add_report(make_report("rep-good-1", text="fine one"))
        # No "id" at all forces a real failure in sync_report's validation.
        self.worker_state.reports["rep-bad"] = {"text": "no id field"}
        self.worker_state.add_report(make_report("rep-good-2", text="also fine"))

        rc, _ = self._capture_run()
        self.assertEqual(rc, 1)  # non-zero exit since one report failed

        notes = {n["path"] for n in self.vault.tree()["notes"]}
        self.assertIn("Storm/Feedback/Inbox/FB-0001.md", notes)
        self.assertIn("Storm/Feedback/Inbox/FB-0002.md", notes)
        # The bad report was never deleted — it is still pending next run.
        self.assertIn("rep-bad", self.worker_state.reports)

    def test_nothing_sensitive_appears_in_logs(self):
        secret_text = "my email is secret@example.com and here is a token stk_SECRETVALUE"
        self.worker_state.add_report(
            make_report(
                "rep-7", text=secret_text, email="reporter@example.com",
                bundle={"api_key": "super-secret-bundle-value"},
            )
        )
        self.worker_state.screenshots["rep-7"] = b"PNGDATA"
        rc, output = self._capture_run()
        self.assertEqual(rc, 0)

        self.assertNotIn("secret@example.com", output)
        self.assertNotIn("stk_SECRETVALUE", output)
        self.assertNotIn("reporter@example.com", output)
        self.assertNotIn("super-secret-bundle-value", output)
        self.assertNotIn(self.worker_state.token, output)
        self.assertNotIn(self.storm_state.access_token, output)
        self.assertNotIn(self.storm_state.refresh_token, output)

        # Every logged line must be a small JSON record of ids/counts.
        for line in output.strip().splitlines():
            record = json.loads(line)
            self.assertIn("event", record)


class IdempotencyTests(FakeServersTestCase):
    def test_crash_after_note_create_but_before_delete_via_state_file(self):
        self.worker_state.add_report(make_report("rep-crash-1", text="about to crash"))

        # Simulates a first run that got partway: note created and recorded,
        # then "crashed" before the Worker delete.
        state = fs.State.load(self.state_dir)
        inbox = fs.InboxIndex(self.cfg, self.storm_state.access_token)
        report = dict(self.worker_state.reports["rep-crash-1"])
        report["has_screenshot"] = False
        ref = fs.allocate_ref(state, inbox)
        path = f"{self.cfg.feedback_folder}/Inbox/{ref}.md"
        fs.api_create_note(
            self.cfg.storm_url, self.storm_state.access_token, self.cfg.vault, path,
            fs.render_note(ref, report),
        )
        state.synced["rep-crash-1"] = ref
        state.save()

        notes_before = len(self.vault.notes)

        rc, _ = self._capture_run()
        self.assertEqual(rc, 0)

        self.assertEqual(len(self.vault.notes), notes_before)
        self.assertNotIn("rep-crash-1", self.worker_state.reports)

    def test_crash_recovered_via_vault_scan_when_state_is_lost(self):
        self.worker_state.add_report(make_report("rep-crash-2", text="about to lose state"))

        state = fs.State.load(self.state_dir)
        inbox = fs.InboxIndex(self.cfg, self.storm_state.access_token)
        report = dict(self.worker_state.reports["rep-crash-2"])
        report["has_screenshot"] = False
        ref = fs.allocate_ref(state, inbox)
        path = f"{self.cfg.feedback_folder}/Inbox/{ref}.md"
        fs.api_create_note(
            self.cfg.storm_url, self.storm_state.access_token, self.cfg.vault, path,
            fs.render_note(ref, report),
        )
        state.synced["rep-crash-2"] = ref
        state.save()

        # The state file itself is gone; the rerun has only the note to go on.
        os.remove(fs.State._path(self.state_dir))

        notes_before = len(self.vault.notes)
        rc, _ = self._capture_run()
        self.assertEqual(rc, 0)

        self.assertEqual(len(self.vault.notes), notes_before)
        self.assertNotIn("rep-crash-2", self.worker_state.reports)

    def test_crash_after_note_create_but_before_feedback_md_insert(self):
        self.worker_state.add_report(make_report("rep-crash-3", text="crash before the bullet"))

        # Simulates a first run that crashed before `_finish_sync` ever ran.
        state = fs.State.load(self.state_dir)
        inbox = fs.InboxIndex(self.cfg, self.storm_state.access_token)
        report = dict(self.worker_state.reports["rep-crash-3"])
        report["has_screenshot"] = False
        ref = fs.allocate_ref(state, inbox)
        path = f"{self.cfg.feedback_folder}/Inbox/{ref}.md"
        fs.api_create_note(
            self.cfg.storm_url, self.storm_state.access_token, self.cfg.vault, path,
            fs.render_note(ref, report),
        )
        state.synced["rep-crash-3"] = ref
        state.save()

        content_before = self._feedback_md_content()
        self.assertNotIn(ref, content_before)
        notes_before = len(self.vault.notes)

        rc, _ = self._capture_run()
        self.assertEqual(rc, 0)

        self.assertEqual(len(self.vault.notes), notes_before)
        # Counts the whole wikilink, not the bare ref, since `ref` appears
        # twice per bullet (path and display text).
        content_after = self._feedback_md_content()
        link = f"[[{self.cfg.feedback_folder}/Inbox/{ref}|{ref}]]"
        self.assertEqual(content_after.count(link), 1)
        self.assertNotIn("rep-crash-3", self.worker_state.reports)

    def test_rerun_with_nothing_pending_is_a_silent_no_op(self):
        rc, _ = self._capture_run()
        self.assertEqual(rc, 0)
        self.assertEqual(len(self.vault.notes), 1)  # just the seeded Feedback.md


class SessionRefreshTests(FakeServersTestCase):
    def test_expired_access_token_is_refreshed_before_use(self):
        creds = fs.DeviceCreds.load(self.state_dir)
        creds.access_expires = "2000-01-01T00:00:00Z"  # long past
        creds.save(self.state_dir)

        self.worker_state.add_report(make_report("rep-refresh", text="needs a refresh"))
        rc, _ = self._capture_run()
        self.assertEqual(rc, 0)
        self.assertGreaterEqual(self.storm_state.refresh_calls, 1)

        reloaded = fs.DeviceCreds.load(self.state_dir)
        self.assertNotEqual(reloaded.access_token, "access-stale-does-not-exist")
        self.assertTrue(reloaded.access_token)

    def test_fresh_access_token_is_not_refreshed(self):
        creds = fs.DeviceCreds.load(self.state_dir)
        creds.access_expires = "2099-01-01T00:00:00Z"
        creds.save(self.state_dir)

        self.worker_state.add_report(make_report("rep-no-refresh"))
        rc, _ = self._capture_run()
        self.assertEqual(rc, 0)
        self.assertEqual(self.storm_state.refresh_calls, 0)


class NotPairedTests(unittest.TestCase):
    def test_run_without_device_creds_fails_cleanly(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            cfg = fs.Config(
                storm_url="http://127.0.0.1:1", vault="v", feedback_folder="Storm/Feedback",
                intake_url="http://127.0.0.1:1", sync_token="t", state_dir=tmp,
            )
            rc = fs.run(cfg)
            self.assertEqual(rc, 1)


if __name__ == "__main__":
    unittest.main()
