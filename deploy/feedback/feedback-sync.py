#!/usr/bin/env python3
"""The feedback sync service: pulls pending reports from the intake Worker and files them into the vault as Storm/Feedback/Inbox/FB-####.md, as its own paired Storm device. Pure stdlib Python (no pip on the prod VM)."""

import argparse
import base64
import dataclasses
import datetime
import getpass
import hashlib
import json
import os
import re
import secrets
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request

VERSION = "162-6.1"
USER_AGENT = "storm-feedback-sync/" + VERSION

DEFAULT_STATE_DIR = "/var/lib/storm-feedback-sync"
DEFAULT_FEEDBACK_FOLDER = "Storm/Feedback"

FEEDBACK_MD_INBOX_HEADING = "## Inbox (new)"
FEEDBACK_MD_PLACEHOLDER = "_None yet._"

REF_RE = re.compile(r"^FB-(\d+)$")


# ---------------------------------------------------------------------------
# Pure-Python Ed25519 (RFC 8032), verify only — no `cryptography` package on
# the prod VM. Iterative scalar multiplication, not recursive: a 512-bit
# multiplication recursed naively gets close to Python's recursion limit.
# ---------------------------------------------------------------------------

_ED_Q = 2**255 - 19
_ED_L = 2**252 + 27742317777372353535851937790883648493
_ED_BITS = 256


def _ed_inv(x):
    return pow(x, _ED_Q - 2, _ED_Q)


_ED_D = (-121665 * _ed_inv(121666)) % _ED_Q
_ED_I = pow(2, (_ED_Q - 1) // 4, _ED_Q)


def _ed_xrecover(y):
    xx = (y * y - 1) * _ed_inv(_ED_D * y * y + 1) % _ED_Q
    x = pow(xx, (_ED_Q + 3) // 8, _ED_Q)
    if (x * x - xx) % _ED_Q != 0:
        x = (x * _ED_I) % _ED_Q
    if x % 2 != 0:
        x = _ED_Q - x
    return x


_ED_BY = (4 * _ed_inv(5)) % _ED_Q
_ED_BX = _ed_xrecover(_ED_BY)
_ED_B = (_ED_BX, _ED_BY)


def _ed_edwards(p, q):
    x1, y1 = p
    x2, y2 = q
    x3 = (x1 * y2 + x2 * y1) * _ed_inv(1 + _ED_D * x1 * x2 * y1 * y2)
    y3 = (y1 * y2 + x1 * x2) * _ed_inv(1 - _ED_D * x1 * x2 * y1 * y2)
    return (x3 % _ED_Q, y3 % _ED_Q)


def _ed_scalarmult(p, e):
    q = (0, 1)
    while e > 0:
        if e & 1:
            q = _ed_edwards(q, p)
        p = _ed_edwards(p, p)
        e >>= 1
    return q


def _ed_encodeint(y):
    return y.to_bytes(_ED_BITS // 8, "little")


def _ed_decodeint(s):
    return int.from_bytes(s, "little")


def _ed_encodepoint(p):
    x, y = p
    out = bytearray(y.to_bytes(_ED_BITS // 8, "little"))
    if x & 1:
        out[-1] |= 0x80
    return bytes(out)


def _ed_isoncurve(p):
    x, y = p
    return (-x * x + y * y - 1 - _ED_D * x * x * y * y) % _ED_Q == 0


def _ed_decodepoint(s):
    y = _ed_decodeint(s) & ((1 << (_ED_BITS - 1)) - 1)
    sign = (s[-1] >> 7) & 1
    x = _ed_xrecover(y)
    if (x & 1) != sign:
        x = _ED_Q - x
    p = (x, y)
    if not _ed_isoncurve(p):
        raise ValueError("decoding point that is not on curve")
    return p


def ed25519_verify(signature, message, public_key):
    """Verifies a raw 64-byte Ed25519 signature; returns False (never raises) on malformed input."""
    try:
        if len(signature) != _ED_BITS // 4 or len(public_key) != _ED_BITS // 8:
            return False
        r_enc = signature[: _ED_BITS // 8]
        s_enc = signature[_ED_BITS // 8 :]
        s = _ed_decodeint(s_enc)
        if s >= _ED_L:
            return False
        r = _ed_decodepoint(r_enc)
        a = _ed_decodepoint(public_key)
        h = hashlib.sha512(r_enc + public_key + message).digest()
        hint = _ed_decodeint(h)
        x1, y1 = _ed_scalarmult(_ED_B, s)
        x2, y2 = _ed_edwards(r, _ed_scalarmult(a, hint))
        return x1 == x2 and y1 == y2
    except Exception:
        return False


_B64URL_RE = re.compile(r"^[A-Za-z0-9_-]*$")


def b64url_decode(s):
    """Strict unpadded base64url; rejects the standard +/ alphabet, which `base64.urlsafe_b64decode` alone would accept and let a key decode two ways."""
    s = s.strip()
    if not _B64URL_RE.match(s):
        raise ValueError("not valid unpadded base64url")
    pad = "=" * (-len(s) % 4)
    return base64.urlsafe_b64decode(s + pad)


def b64url_encode(b):
    return base64.urlsafe_b64encode(b).decode("ascii").rstrip("=")


# ---------------------------------------------------------------------------
# HTTP
# ---------------------------------------------------------------------------


class ApiError(Exception):
    def __init__(self, status, message):
        super().__init__(f"HTTP {status}: {message}")
        self.status = status
        self.message = message


def http_request(method, url, headers=None, data=None, timeout=20):
    """Returns (status, headers_dict, body_bytes); raises only on a transport failure, never an HTTP error status."""
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("User-Agent", USER_AGENT)
    for k, v in (headers or {}).items():
        req.add_header(k, v)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, dict(resp.headers.items()), resp.read()
    except urllib.error.HTTPError as e:
        body = e.read()
        hdrs = dict(e.headers.items()) if e.headers else {}
        return e.code, hdrs, body
    except urllib.error.URLError as e:
        raise ApiError(0, str(e.reason)) from e


def http_json(method, url, headers=None, json_body=None, timeout=20):
    data = None
    hdrs = dict(headers or {})
    if json_body is not None:
        data = json.dumps(json_body).encode("utf-8")
        hdrs.setdefault("Content-Type", "application/json")
    status, resp_headers, body = http_request(method, url, hdrs, data, timeout)
    parsed = None
    if body:
        try:
            parsed = json.loads(body.decode("utf-8"))
        except (ValueError, UnicodeDecodeError):
            parsed = None
    return status, parsed, resp_headers


def _err_msg(body):
    if isinstance(body, dict) and "error" in body:
        return str(body["error"])
    return "unexpected response"


def _ok(status):
    return 200 <= status < 300


def _quote_path(path):
    return "/".join(urllib.parse.quote(seg, safe="") for seg in path.split("/"))


# ---------------------------------------------------------------------------
# Storm server REST calls
# ---------------------------------------------------------------------------


def api_server_info(server_url):
    status, body, _ = http_json("GET", f"{server_url}/v1/server")
    if not _ok(status) or not isinstance(body, dict):
        raise ApiError(status, _err_msg(body))
    return body


def api_challenge(server_url, nonce):
    status, body, _ = http_json(
        "POST", f"{server_url}/v1/server/challenge", json_body={"nonce": nonce}
    )
    if not _ok(status) or not isinstance(body, dict):
        raise ApiError(status, _err_msg(body))
    return body


def api_pair(server_url, nonce, name, platform, version):
    status, body, _ = http_json(
        "POST",
        f"{server_url}/v1/pair",
        json_body={"n": nonce, "name": name, "platform": platform, "version": version},
    )
    if not _ok(status) or not isinstance(body, dict):
        raise ApiError(status, _err_msg(body))
    return body


def api_login(server_url, device_id, device_secret, password):
    headers = {"Authorization": f"StormDevice {device_id}:{device_secret}"}
    status, body, _ = http_json(
        "POST", f"{server_url}/v1/auth/login", headers=headers, json_body={"password": password}
    )
    if not _ok(status) or not isinstance(body, dict):
        raise ApiError(status, _err_msg(body))
    return body


def api_refresh(server_url, refresh_token):
    status, body, _ = http_json(
        "POST", f"{server_url}/v1/auth/refresh", json_body={"refresh_token": refresh_token}
    )
    if not _ok(status) or not isinstance(body, dict):
        raise ApiError(status, _err_msg(body))
    return body


def _auth_headers(access_token):
    return {"Authorization": f"Bearer {access_token}"}


def api_tree(server_url, access_token, vault):
    status, body, _ = http_json(
        "GET", f"{server_url}/v1/vaults/{vault}/tree", headers=_auth_headers(access_token)
    )
    if not _ok(status) or not isinstance(body, dict):
        raise ApiError(status, _err_msg(body))
    return body


def api_get_note(server_url, access_token, vault, note_id):
    status, body, _ = http_json(
        "GET",
        f"{server_url}/v1/vaults/{vault}/notes/{urllib.parse.quote(note_id, safe='')}",
        headers=_auth_headers(access_token),
    )
    if not _ok(status) or not isinstance(body, dict):
        raise ApiError(status, _err_msg(body))
    return body


def api_create_note(server_url, access_token, vault, path, content):
    status, body, _ = http_json(
        "POST",
        f"{server_url}/v1/vaults/{vault}/notes",
        headers=_auth_headers(access_token),
        json_body={"path": path, "content": content},
    )
    if not _ok(status) or not isinstance(body, dict):
        raise ApiError(status, _err_msg(body))
    return body


def api_put_note(server_url, access_token, vault, note_id, base_version, content):
    status, body, _ = http_json(
        "PUT",
        f"{server_url}/v1/vaults/{vault}/notes/{urllib.parse.quote(note_id, safe='')}",
        headers=_auth_headers(access_token),
        json_body={"base_version": base_version, "content": content},
    )
    if not _ok(status) or not isinstance(body, dict):
        raise ApiError(status, _err_msg(body))
    return body


def api_put_attachment(server_url, access_token, vault, path, data):
    url = f"{server_url}/v1/vaults/{vault}/attachments/{_quote_path(path)}"
    headers = dict(_auth_headers(access_token))
    headers["Content-Type"] = "application/octet-stream"
    status, _, body = http_request("PUT", url, headers=headers, data=data)
    parsed = None
    if body:
        try:
            parsed = json.loads(body.decode("utf-8"))
        except (ValueError, UnicodeDecodeError):
            parsed = None
    if not _ok(status):
        raise ApiError(status, _err_msg(parsed))
    return parsed


# ---------------------------------------------------------------------------
# Intake Worker calls
# ---------------------------------------------------------------------------


def worker_list_reports(intake_url, token, limit=100):
    status, body, _ = http_json(
        "GET",
        f"{intake_url}/v1/reports?limit={limit}",
        headers={"Authorization": f"Bearer {token}"},
    )
    if not _ok(status) or not isinstance(body, dict):
        raise ApiError(status, _err_msg(body))
    return body.get("reports", [])


def worker_get_screenshot(intake_url, token, report_id):
    status, _, body = http_request(
        "GET",
        f"{intake_url}/v1/reports/{urllib.parse.quote(report_id, safe='')}/screenshot",
        headers={"Authorization": f"Bearer {token}"},
    )
    if status == 404:
        return None
    if not _ok(status):
        raise ApiError(status, "GET screenshot failed")
    return body


def worker_delete_report(intake_url, token, report_id):
    status, _, body = http_request(
        "DELETE",
        f"{intake_url}/v1/reports/{urllib.parse.quote(report_id, safe='')}",
        headers={"Authorization": f"Bearer {token}"},
    )
    if status == 404:
        return  # already gone — fine, that's the point of idempotency.
    if not _ok(status):
        raise ApiError(status, "DELETE report failed")


# ---------------------------------------------------------------------------
# Pairing code (storm://pair URI, or a bare nonce)
# ---------------------------------------------------------------------------


class PairingError(Exception):
    pass


def parse_pairing_code(code):
    """Parses `--code`: a `storm://pair?...` URI, or a bare nonce. Whitespace is stripped first since this is typed or pasted by hand."""
    cleaned = re.sub(r"\s+", "", code or "")
    if not cleaned:
        raise PairingError("empty pairing code")
    if "://" in cleaned:
        if not cleaned.startswith("storm://"):
            raise PairingError("not a storm:// pairing URI")
        parsed = urllib.parse.urlparse(cleaned)
        if parsed.scheme != "storm" or parsed.netloc != "pair":
            raise PairingError("not a storm://pair URI")
        q = urllib.parse.parse_qs(parsed.query)

        def one(key):
            vals = q.get(key)
            return vals[0] if vals else None

        nonce = one("n")
        if not nonce:
            raise PairingError("pairing URI is missing n=")
        return {
            "nonce": nonce,
            "server_id": one("sid"),
            "public_key": one("pk"),
        }
    return {"nonce": cleaned, "server_id": None, "public_key": None}


def perform_pairing(server_url, code, name, platform, version, password):
    """Runs the full handshake: verify the server's identity by challenge-response, consume the pairing nonce, then log in once for a refresh token. Not retried; a one-time manual step."""
    parsed = parse_pairing_code(code)
    info = api_server_info(server_url)
    server_id = info.get("server_id")
    if parsed["server_id"] and parsed["server_id"] != server_id:
        raise PairingError(
            "the pairing code is for a different server than the one at --server"
        )

    nonce = secrets.token_urlsafe(24)
    answer = api_challenge(server_url, nonce)
    if answer.get("server_id") != server_id:
        raise PairingError("challenge answered by a different server id")

    try:
        sig = b64url_decode(answer["signature"])
        pubkey = b64url_decode(info["public_key"])
    except Exception as e:
        raise PairingError("malformed challenge response") from e

    message = f"storm-challenge:v1:{server_id}:{nonce}".encode("utf-8")
    if not ed25519_verify(sig, message, pubkey):
        raise PairingError(
            "the server at --server could not prove it holds the key named "
            "in the pairing code — refusing to pair (possible MITM or wrong "
            "server)"
        )

    consumed = api_pair(server_url, parsed["nonce"], name, platform, version)
    issued = api_login(server_url, consumed["device_id"], consumed["device_secret"], password)

    return DeviceCreds(
        device_id=consumed["device_id"],
        device_secret=consumed["device_secret"],
        server_id=consumed["server_id"],
        access_token=issued["access_token"],
        access_expires=issued.get("expires", ""),
        refresh_token=issued["refresh_token"],
        refresh_expires=issued.get("refresh_expires", ""),
    )


# ---------------------------------------------------------------------------
# State on disk — device credential and the synced-report map.
#
# Both files are written atomically (temp file + rename) and mode 0600, in a
# 0700 directory: the device secret and the refresh token are long-lived
# credentials.
# ---------------------------------------------------------------------------


def _ensure_state_dir(state_dir):
    os.makedirs(state_dir, exist_ok=True)
    try:
        os.chmod(state_dir, 0o700)
    except OSError:
        pass


def _atomic_write_json(path, data):
    _ensure_state_dir(os.path.dirname(path))
    fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path), prefix=".tmp-")
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(data, f)
            f.write("\n")
        os.chmod(tmp, 0o600)
        # `pair` runs as root; the unit's DynamicUser owns the state dir and
        # must still be able to read what root wrote there.
        if os.geteuid() == 0:
            st = os.stat(os.path.dirname(path))
            if st.st_uid != 0:
                os.chown(tmp, st.st_uid, st.st_gid)
        os.replace(tmp, path)
    except Exception:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


@dataclasses.dataclass
class DeviceCreds:
    device_id: str
    device_secret: str
    server_id: str
    access_token: str = ""
    access_expires: str = ""
    refresh_token: str = ""
    refresh_expires: str = ""

    @staticmethod
    def _path(state_dir):
        return os.path.join(state_dir, "device.json")

    @classmethod
    def load(cls, state_dir):
        p = cls._path(state_dir)
        if not os.path.exists(p):
            return None
        with open(p, "r") as f:
            data = json.load(f)
        fields = (
            "device_id",
            "device_secret",
            "server_id",
            "access_token",
            "access_expires",
            "refresh_token",
            "refresh_expires",
        )
        return cls(**{k: data.get(k, "") for k in fields})

    def save(self, state_dir):
        _atomic_write_json(self._path(state_dir), dataclasses.asdict(self))


@dataclasses.dataclass
class State:
    """The idempotency map: intake report id -> FB ref, plus the next ref number. `next_ref` starts None so a fresh state dir computes it from the vault rather than assuming FB-0001."""

    synced: dict
    next_ref: object
    state_dir: str

    @staticmethod
    def _path(state_dir):
        return os.path.join(state_dir, "state.json")

    @classmethod
    def load(cls, state_dir):
        p = cls._path(state_dir)
        if os.path.exists(p):
            with open(p, "r") as f:
                data = json.load(f)
            return cls(
                synced=dict(data.get("synced", {})),
                next_ref=data.get("next_ref"),
                state_dir=state_dir,
            )
        return cls(synced={}, next_ref=None, state_dir=state_dir)

    def save(self):
        _atomic_write_json(
            self._path(self.state_dir), {"synced": self.synced, "next_ref": self.next_ref}
        )


# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------


@dataclasses.dataclass
class Config:
    storm_url: str
    vault: str
    feedback_folder: str
    intake_url: str
    sync_token: str
    state_dir: str


def load_config(env=None):
    env = env if env is not None else os.environ

    def require(key):
        v = env.get(key)
        if not v:
            raise SystemExit(f"{key} is required (set it in the environment or env file)")
        return v

    return Config(
        storm_url=require("STORM_URL").rstrip("/"),
        vault=require("STORM_VAULT"),
        feedback_folder=env.get("FEEDBACK_FOLDER", DEFAULT_FEEDBACK_FOLDER).rstrip("/"),
        intake_url=require("INTAKE_URL").rstrip("/"),
        sync_token=require("SYNC_TOKEN"),
        state_dir=env.get("STATE_DIR", DEFAULT_STATE_DIR),
    )


def load_env_file(path):
    """A minimal KEY=VALUE reader for --env-file, so `pair` and manual runs can point at the same file systemd's EnvironmentFile= uses."""
    with open(path, "r") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if "=" not in line:
                continue
            key, _, value = line.partition("=")
            key = key.strip()
            value = value.strip()
            if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
                value = value[1:-1]
            os.environ[key] = value


# ---------------------------------------------------------------------------
# Logging — ids and counts only, never report content.
# ---------------------------------------------------------------------------


def log(event, **fields):
    print(json.dumps({"event": event, **fields}, sort_keys=True), flush=True)


# ---------------------------------------------------------------------------
# Session lifecycle
# ---------------------------------------------------------------------------

# Access tokens last 30 days; refresh well before that so a box that's been
# off for a few days doesn't come back to a token that expired mid-run.
_REFRESH_MARGIN = datetime.timedelta(hours=12)


def _parse_rfc3339(s):
    if s.endswith("Z"):
        s = s[:-1] + "+00:00"
    m = re.match(r"^(.*?)(\.\d+)?([+-]\d{2}:\d{2})$", s)
    if m and m.group(2):
        frac = m.group(2)[1:][:6]
        s = m.group(1) + "." + frac + m.group(3)
    return datetime.datetime.fromisoformat(s)


def obtain_session(cfg, creds):
    """Returns a valid access token, refreshing first if close to expiry.

    A refresh invalidates the old refresh token, so the new one is persisted
    immediately on success, before anything else uses it.
    """
    needs_refresh = True
    if creds.access_token and creds.access_expires:
        try:
            exp = _parse_rfc3339(creds.access_expires)
            now = datetime.datetime.now(datetime.timezone.utc)
            needs_refresh = (exp - now) < _REFRESH_MARGIN
        except ValueError:
            needs_refresh = True

    if not needs_refresh:
        return creds.access_token

    issued = api_refresh(cfg.storm_url, creds.refresh_token)
    creds.access_token = issued["access_token"]
    creds.refresh_token = issued["refresh_token"]
    creds.access_expires = issued.get("expires", "")
    creds.refresh_expires = issued.get("refresh_expires", "")
    creds.save(cfg.state_dir)
    log("session_refreshed")
    return creds.access_token


# ---------------------------------------------------------------------------
# Note templating
# ---------------------------------------------------------------------------


def _first_line_raw(text):
    if not text:
        return ""
    for line in text.splitlines():
        return line
    return ""


def first_line(text, limit=80):
    return _first_line_raw(text)[:limit]


def short_title(text, limit=60):
    line = _first_line_raw(text)
    if len(line) <= limit:
        return line
    return line[: limit - 1].rstrip() + "…"


def fenced(text, lang=""):
    """Wraps `text` in a fence long enough that nothing inside it can close it early."""
    longest_run = 0
    run = 0
    for ch in text:
        if ch == "`":
            run += 1
            longest_run = max(longest_run, run)
        else:
            run = 0
    fence = "`" * max(3, longest_run + 1)
    return f"{fence}{lang}\n{text}\n{fence}"


def _stringify(value):
    if value is None:
        return ""
    if isinstance(value, str):
        return value
    if isinstance(value, (list, tuple)):
        return ", ".join(_stringify(v) for v in value)
    try:
        return json.dumps(value, ensure_ascii=False)
    except TypeError:
        return str(value)


def render_context(client):
    """`client` is the caller's own free-form shape — read defensively, since nothing here is guaranteed to exist or to be a string."""
    client = client if isinstance(client, dict) else {}
    screen = _stringify(client.get("screen")) or "—"
    actions = _stringify(client.get("last_actions")) or "—"
    error = _stringify(client.get("error")) or "—"
    return (
        f"- Screen: {screen}\n"
        f"- Last actions: {actions}\n"
        f'- Error (from "Report this"): {error}'
    )


def render_note(ref, report):
    """Builds the note body. The server stamps `id`, `created` and `modified` itself — this content carries none of them."""
    client = report.get("client") if isinstance(report.get("client"), dict) else {}
    bundle = report.get("bundle") if isinstance(report.get("bundle"), dict) else {}
    version = client.get("version") or "unknown"
    platform = client.get("platform") or "unknown"
    kind = report.get("kind") or "other"
    text = report.get("text") or ""
    email = report.get("email") or ""
    received = report.get("received_at") or ""
    intake_id = report.get("id") or ""
    title = first_line(text, 80)

    frontmatter = (
        "---\n"
        "type: report\n"
        f"ref: {ref}\n"
        f"intake: {intake_id}\n"
        "status: new\n"
        f"kind: {kind}\n"
        f"received: {received}\n"
        f"version: {version}\n"
        f"platform: {platform}\n"
        "issue:\n"
        f"reply_to: {email}\n"
        "---\n"
    )

    diagnostics = fenced(json.dumps(bundle, indent=2, ensure_ascii=False, sort_keys=True), "text")

    parts = [
        frontmatter,
        f"\n# {ref} — {title}\n",
        "## What happened\n" + text + "\n",
        "## Context\n" + render_context(client) + "\n",
        "## Attachments\n"
        + (f"- Screenshot: ![[{ref}.png]]" if report.get("has_screenshot") else "- Screenshot: none")
        + "\n",
        "## Diagnostics\n" + diagnostics + "\n",
        "## Triage\n- [ ] Duplicate checked\n"
        "- [ ] Linked to an Issue / Idea, or declined with a reason\n",
    ]
    return "\n".join(parts)


def feedback_md_line(ref, folder, kind, title, version):
    return f'- [ ] [[{folder}/Inbox/{ref}|{ref}]] · {kind} · "{title}" · {version}'


def update_feedback_md_content(content, heading, ref, bullet):
    """Inserts `bullet` under `heading`, removing a lone `_None yet._` placeholder. A no-op if `ref` is already there, which is what makes a retried update safe."""
    pattern = re.compile(
        r"(^" + re.escape(heading) + r"[ \t]*\n)(.*?)(?=^#{1,6}[ \t]|\Z)",
        re.MULTILINE | re.DOTALL,
    )
    m = pattern.search(content)
    if not m:
        sep = "" if content.endswith("\n\n") else ("\n" if content.endswith("\n") else "\n\n")
        return content + f"{sep}{heading}\n\n{bullet}\n"

    body = m.group(2)
    if ref in body:
        return content

    body_lines = [l for l in body.split("\n") if l.strip() != FEEDBACK_MD_PLACEHOLDER]
    text = "\n".join(body_lines).strip("\n")
    new_body = (text + "\n" if text else "") + bullet + "\n\n"
    return content[: m.start(2)] + new_body + content[m.end(2) :]


def update_feedback_index(cfg, access_token, ref, kind, title, version, max_attempts=5):
    """Read-modify-write `Feedback.md` under its own base version, retrying on a merge conflict."""
    note_path = f"{cfg.feedback_folder}/Feedback.md"
    tree = api_tree(cfg.storm_url, access_token, cfg.vault)
    note = next((n for n in tree.get("notes", []) if n.get("path") == note_path), None)
    if note is None:
        raise RuntimeError(f"{note_path} does not exist in the vault")
    note_id = note["id"]
    bullet = feedback_md_line(ref, cfg.feedback_folder, kind, title, version)

    for attempt in range(max_attempts):
        detail = api_get_note(cfg.storm_url, access_token, cfg.vault, note_id)
        content = detail.get("content", "")
        new_content = update_feedback_md_content(content, FEEDBACK_MD_INBOX_HEADING, ref, bullet)
        if new_content == content:
            return
        result = api_put_note(
            cfg.storm_url, access_token, cfg.vault, note_id, detail["version"], new_content
        )
        if not result.get("conflict"):
            return
        log("feedback_md_conflict_retry", attempt=attempt)
    raise RuntimeError("could not update Feedback.md after retries (merge conflicts)")


# ---------------------------------------------------------------------------
# Inbox scan — numbering, and the idempotency fallback.
# ---------------------------------------------------------------------------


def parse_frontmatter_scalar(content, key):
    if content.startswith("﻿"):
        content = content[1:]
    lines = content.split("\n")
    if not lines or lines[0].strip() != "---":
        return None
    prefix = f"{key}:"
    for line in lines[1:]:
        if line.strip() == "---":
            break
        if line.startswith(prefix):
            value = line[len(prefix) :].strip()
            return value or None
    return None


def scan_inbox(cfg, access_token):
    """Every note under `<folder>/Inbox/`, keyed by note id, with its `ref` and `intake` frontmatter values — used to allocate the next ref number and as the idempotency fallback."""
    tree = api_tree(cfg.storm_url, access_token, cfg.vault)
    prefix = f"{cfg.feedback_folder}/Inbox/"
    out = {}
    for n in tree.get("notes", []):
        path = n.get("path", "")
        if not path.startswith(prefix) or not path.endswith(".md"):
            continue
        detail = api_get_note(cfg.storm_url, access_token, cfg.vault, n["id"])
        content = detail.get("content", "")
        out[n["id"]] = {
            "path": path,
            "ref": parse_frontmatter_scalar(content, "ref"),
            "intake": parse_frontmatter_scalar(content, "intake"),
        }
    return out


class InboxIndex:
    """Scans the vault's Inbox folder at most once per run."""

    def __init__(self, cfg, access_token):
        self._cfg = cfg
        self._access_token = access_token
        self._data = None

    def _ensure(self):
        if self._data is None:
            self._data = scan_inbox(self._cfg, self._access_token)
        return self._data

    def find_by_intake(self, intake_id):
        for info in self._ensure().values():
            if info.get("intake") == intake_id:
                return info.get("ref")
        return None

    def max_ref_number(self):
        best = 0
        for info in self._ensure().values():
            ref = info.get("ref")
            m = REF_RE.match(ref) if ref else None
            if m:
                best = max(best, int(m.group(1)))
        return best


def allocate_ref(state, inbox):
    if state.next_ref is None:
        state.next_ref = inbox.max_ref_number() + 1
    n = state.next_ref
    state.next_ref = n + 1
    state.save()
    return f"FB-{n:04d}"


# ---------------------------------------------------------------------------
# Syncing one report
# ---------------------------------------------------------------------------


def _finish_sync(cfg, access_token, ref, report):
    """The screenshot attachment and the `Feedback.md` line. Both are idempotent, so this safely runs again on a crashed earlier run's leftovers."""
    if report.get("has_screenshot"):
        report_id = report.get("id")
        png = worker_get_screenshot(cfg.intake_url, cfg.sync_token, report_id)
        if png:
            api_put_attachment(
                cfg.storm_url,
                access_token,
                cfg.vault,
                f"{cfg.feedback_folder}/Inbox/{ref}.png",
                png,
            )

    client = report.get("client") if isinstance(report.get("client"), dict) else {}
    title = short_title(report.get("text") or "")
    version = client.get("version") or "unknown"
    update_feedback_index(cfg, access_token, ref, report.get("kind") or "other", title, version)


def sync_report(cfg, access_token, state, inbox, report):
    report_id = report.get("id")
    if not report_id:
        raise ValueError("report has no id")

    ref = state.synced.get(report_id)
    if ref is None:
        ref = inbox.find_by_intake(report_id)
        if ref is not None:
            state.synced[report_id] = ref
            state.save()

    if ref is None:
        ref = allocate_ref(state, inbox)
        path = f"{cfg.feedback_folder}/Inbox/{ref}.md"
        content = render_note(ref, report)
        api_create_note(cfg.storm_url, access_token, cfg.vault, path, content)

        # Recorded right away: if anything below fails, the next run finds
        # this note and redoes only the idempotent steps that follow, never
        # the create.
        state.synced[report_id] = ref
        state.save()
        log("report_note_created", id=report_id, ref=ref)
    else:
        log("report_already_synced", id=report_id, ref=ref)

    _finish_sync(cfg, access_token, ref, report)
    worker_delete_report(cfg.intake_url, cfg.sync_token, report_id)
    log("report_synced", id=report_id, ref=ref, has_screenshot=bool(report.get("has_screenshot")))


def run(cfg):
    creds = DeviceCreds.load(cfg.state_dir)
    if creds is None:
        log("not_paired")
        print(
            "not paired — run: feedback-sync.py pair --server <url> --code <code>",
            file=sys.stderr,
        )
        return 1

    try:
        access_token = obtain_session(cfg, creds)
    except ApiError as e:
        log("session_failed", error_type=type(e).__name__, status=e.status)
        return 1

    state = State.load(cfg.state_dir)
    inbox = InboxIndex(cfg, access_token)

    try:
        reports = worker_list_reports(cfg.intake_url, cfg.sync_token, limit=100)
    except Exception as e:
        log("worker_list_failed", error_type=type(e).__name__)
        return 1

    log("reports_fetched", count=len(reports))
    failed = 0
    for report in reports:
        rid = report.get("id", "?") if isinstance(report, dict) else "?"
        try:
            sync_report(cfg, access_token, state, inbox, report)
        except Exception as e:
            failed += 1
            log("report_sync_failed", id=rid, error_type=type(e).__name__)

    log("run_complete", total=len(reports), failed=failed)
    return 1 if failed else 0


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def _read_password(args):
    if args.password_stdin:
        return sys.stdin.readline().rstrip("\n")
    return getpass.getpass("Storm account password: ")


def cmd_pair(args):
    state_dir = args.state_dir or os.environ.get("STATE_DIR", DEFAULT_STATE_DIR)
    password = _read_password(args)
    try:
        creds = perform_pairing(
            args.server.rstrip("/"), args.code, args.name, args.platform, VERSION, password
        )
    except (PairingError, ApiError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    finally:
        password = None

    creds.save(state_dir)
    print(f"paired as device {creds.device_id} with server {creds.server_id}")
    print(f"credentials stored in {state_dir} (mode 0600)")
    return 0


def cmd_run(args):
    cfg = load_config()
    return run(cfg)


def build_arg_parser():
    p = argparse.ArgumentParser(
        prog="feedback-sync.py",
        description="Decision 162-6: sync in-app feedback reports into the vault.",
    )
    p.add_argument(
        "--env-file", help="load KEY=VALUE pairs from this file into the environment first"
    )
    sub = p.add_subparsers(dest="command", required=True)

    pair_p = sub.add_parser("pair", help="pair this box as a Storm device, once")
    pair_p.add_argument(
        "--server", required=True, help="the Storm server's base URL, e.g. http://host:8484"
    )
    pair_p.add_argument(
        "--code",
        required=True,
        help="a storm://pair URI (Settings > Devices & access > Add a device) or a bare nonce",
    )
    pair_p.add_argument("--name", default="feedback-sync")
    pair_p.add_argument("--platform", default="linux")
    pair_p.add_argument(
        "--password-stdin",
        action="store_true",
        help="read the account password from stdin instead of prompting",
    )
    pair_p.add_argument("--state-dir", help="override STATE_DIR for this call")
    pair_p.set_defaults(func=cmd_pair)

    run_p = sub.add_parser("run", help="pull pending reports once (what the timer runs)")
    run_p.set_defaults(func=cmd_run)

    return p


def main(argv=None):
    args = build_arg_parser().parse_args(argv)
    if args.env_file:
        load_env_file(args.env_file)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
