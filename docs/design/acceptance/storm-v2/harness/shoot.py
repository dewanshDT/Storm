#!/usr/bin/env python3
"""Screenshots of the real Storm web client against a real server.

    python3 shoot.py [--only NAME_PREFIX] [--out DIR] [--keep]

What it does, every run, from nothing:

1. Copies `fixture/vaults` into a fresh temp root and starts the debug
   `storm-server` with the release web bundle (`apps/client/build/web`).
   Build both first: `cargo build` in apps/server, `flutter build web`.
2. Claims the server through the API (pair + first account) so it can
   resolve vault and note ids for routes.
3. Starts headless Chromium and lets the web client bootstrap its own device
   from the page, then signs in **through the UI**, as a person would.
4. For each shot in `shots.py`: sets the viewport (desktop 1280×800, phone
   390×844, both at 2× like the reference PNGs), navigates in-app, runs the
   shot's actions, waits for the frame to settle, captures.

Nothing here fakes data the app shows: the server, the sync, the notes and
the UI are the real ones. The fixture is only the vault content.
"""

import argparse
import base64
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "../../../../.."))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(ROOT, "apps/server/tests"))

import agents  # noqa: E402
import cdp  # noqa: E402
import shots  # noqa: E402
import storm_auth  # noqa: E402

PORT = int(os.environ.get("HARNESS_PORT", "7481"))
DEBUG_PORT = int(os.environ.get("HARNESS_DEBUG_PORT", "9333"))
BASE = f"http://127.0.0.1:{PORT}"
USERNAME = "storm"
PASSWORD = storm_auth.PASSWORD

VIEWPORTS = {
    "desktop": dict(width=1280, height=800, deviceScaleFactor=2, mobile=False),
    "phone": dict(width=390, height=844, deviceScaleFactor=2, mobile=True),
    "gallery": dict(width=1180, height=4600, deviceScaleFactor=1, mobile=False),
}

# Injected once per page: find Flutter semantics nodes by their accessible
# text and report a centre point, so clicks are real pointer events at real
# coordinates rather than synthetic DOM clicks Flutter would ignore.
JS_HELPERS = r"""
window.__storm = {
  enableSemantics() {
    const p = document.querySelector('flt-semantics-placeholder');
    if (p) p.click();
    return !!document.querySelector('flt-semantics-host, flt-semantics');
  },
  nodes() {
    return [...document.querySelectorAll('flt-semantics, flt-semantics input, flt-semantics textarea, input, textarea')];
  },
  label(el) {
    return (el.getAttribute('aria-label') || el.getAttribute('placeholder') ||
            (el.childElementCount === 0 ? el.textContent : '') || '').trim();
  },
  find(pattern) {
    const re = new RegExp(pattern);
    let best = null;
    for (const el of this.nodes()) {
      const l = this.label(el);
      if (!l || !re.test(l)) continue;
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) continue;
      // Something tappable beats a bare label with the same text (a
      // "Sign in" heading above a "Sign in" button); then the smallest wins.
      const tappable = el.getAttribute('role') === 'button' || el.tagName === 'BUTTON' ||
                       el.hasAttribute('flt-tappable') || el.getAttribute('role') === 'link';
      const rank = (tappable ? 0 : 1e9) + r.width * r.height;
      if (!best || rank < best.rank)
        best = {x: r.left + r.width / 2, y: r.top + r.height / 2, rank, label: l, tappable};
    }
    return best;
  },
  inputs() {
    return [...document.querySelectorAll('input, textarea')].map(el => {
      const r = el.getBoundingClientRect();
      return {label: this.label(el), x: r.left + r.width / 2, y: r.top + r.height / 2,
              w: r.width, h: r.height, type: el.type || ''};
    }).filter(i => i.w > 0 && i.h > 0);
  },
};
"""


def log(msg):
    print(f"[harness] {msg}", flush=True)


def wait_for(fn, timeout, what):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            v = fn()
            if v:
                return v
        except Exception:
            pass
        time.sleep(0.25)
    raise RuntimeError(f"timed out waiting for {what}")


class Harness:
    def __init__(self, out_dir, keep):
        self.out = out_dir
        self.keep = keep
        self.tmp = tempfile.mkdtemp(prefix="storm-acceptance-")
        self.procs = []
        self.ids = {}
        self.base = BASE
        self.world = None

    def api(self, method, path, body=None):
        return storm_auth._call(BASE, method, path, body, auth=self.session)

    def step(self, name):
        """Moves the server's agent state on (see agents.py)."""
        self.world = self.world or agents.AgentWorld(self)
        {"hosts": self.world.enroll_hosts,
         "sessions": self.world.run_sessions,
         "loop": self.world.run_loop}[name]()
        log(f"step {name} done")

    # ---- server -------------------------------------------------------

    def start_server(self):
        vaults = os.path.join(self.tmp, "vaults")
        shutil.copytree(os.path.join(HERE, "fixture/vaults"), vaults)
        self.server_log = os.path.join(self.tmp, "server.log")
        binary = os.path.join(ROOT, "apps/server/target/debug/storm-server")
        web = os.path.join(ROOT, "apps/client/build/web")
        for path, hint in [(binary, "cargo build in apps/server"),
                           (os.path.join(web, "index.html"), "flutter build web in apps/client")]:
            if not os.path.exists(path):
                raise SystemExit(f"missing {path} — run {hint}")
        try:
            urllib.request.urlopen(f"{BASE}/v1/health", timeout=1)
            raise SystemExit(f"something already serves {BASE}; set HARNESS_PORT")
        except OSError:
            pass
        fh = open(self.server_log, "w")
        self.procs.append(subprocess.Popen(
            [binary, "serve", "--vault-root", vaults, "--state", os.path.join(self.tmp, "state"),
             "--port", str(PORT), "--web", web, "--mcp"],
            stdout=fh, stderr=subprocess.STDOUT))
        wait_for(lambda: urllib.request.urlopen(f"{BASE}/v1/health", timeout=1).status == 200,
                 30, "the server")
        log(f"server up at {BASE} (state in {self.tmp})")

    def claim(self):
        session, _device, _uid = storm_auth.sign_in(BASE, log_path=self.server_log)
        self.session = session
        _, v = storm_auth._call(BASE, "GET", "/v1/vaults", auth=session)
        for vault in v["vaults"]:
            self.ids[f"vault:{vault['name']}"] = vault["id"]
            _, tree = storm_auth._call(BASE, "GET", f"/v1/vaults/{vault['id']}/tree", auth=session)
            for n in tree["notes"]:
                self.ids[f"note:{vault['name']}/{n['path']}"] = n["id"]
        log(f"claimed; resolved {len(self.ids)} ids")

    def route(self, template):
        """`/v/{vault:personal}/note/{note:personal/projects/storm/BOARD.md}`"""
        out = template
        while "{" in out:
            start = out.index("{")
            end = out.index("}", start)
            key = out[start + 1:end]
            out = out[:start] + self.ids[key] + out[end + 1:]
        return out

    # ---- browser ------------------------------------------------------

    def start_browser(self):
        profile = os.path.join(self.tmp, "chromium")
        self.procs.append(subprocess.Popen(
            ["chromium", "--headless=new", f"--remote-debugging-port={DEBUG_PORT}",
             f"--user-data-dir={profile}", "--no-first-run", "--no-default-browser-check",
             # Chromium's own sandbox cannot start under a parent that set
             # no_new_privs (containers, agent sandboxes). The browser only
             # ever loads the local fixture server, so this costs nothing.
             "--no-sandbox",
             "--hide-scrollbars", "--force-color-profile=srgb", "--window-size=1280,800",
             "about:blank"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        wait_for(lambda: urllib.request.urlopen(f"http://127.0.0.1:{DEBUG_PORT}/json", timeout=1),
                 20, "chromium")
        self.page = cdp.Page(DEBUG_PORT)
        self.page.call("Page.enable")
        self.page.call("Runtime.enable")

    def viewport(self, name):
        vp = VIEWPORTS[name]
        self.page.call("Emulation.setDeviceMetricsOverride", **vp)
        self.page.call("Emulation.setTouchEmulationEnabled", enabled=vp["mobile"])
        self.current_viewport = name

    def location(self):
        return self.page.eval("location.pathname")

    def wait_app(self):
        wait_for(lambda: self.page.eval("!!document.querySelector('flutter-view, flt-glass-pane')"),
                 40, "the Flutter app")
        self.page.eval(JS_HELPERS)
        time.sleep(1.0)

    def settle(self, seconds=1.2):
        time.sleep(seconds)

    def click(self, x, y):
        for kind in ("mousePressed", "mouseReleased"):
            self.page.call("Input.dispatchMouseEvent", type=kind, x=x, y=y,
                           button="left", clickCount=1)
        time.sleep(0.3)

    def tap_text(self, pattern, timeout=10):
        self.page.eval("window.__storm || (" + JS_HELPERS.strip().rstrip(";") + ")")
        self.page.eval("window.__storm.enableSemantics()")
        hit = wait_for(lambda: self.page.eval(f"window.__storm.find({pattern!r})"),
                       timeout, f"something labelled /{pattern}/")
        self.click(hit["x"], hit["y"])
        return hit

    def type_into(self, x, y, text):
        """Tap a field, wait for Flutter to hand it focus, type, and check the
        text arrived — typing before focus lands is silently dropped."""
        self.click(x, y)
        wait_for(lambda: self.page.eval(
            "['INPUT','TEXTAREA'].includes(document.activeElement && document.activeElement.tagName)"),
            5, "a focused text field")
        self.page.call("Input.insertText", text=text)
        wait_for(lambda: self.page.eval("document.activeElement.value") == text,
                 5, "the typed text to land")

    def go(self, path):
        """In-app navigation: go_router follows the browser history."""
        self.page.eval(
            f"history.pushState(null, '', {path!r}); "
            "window.dispatchEvent(new PopStateEvent('popstate', {state: null}))")
        self.settle()

    def sign_in(self):
        self.viewport("desktop")
        self.page.call("Page.navigate", url=BASE + "/")
        self.wait_app()
        wait_for(lambda: self.location() in ("/login", "/pairing") or
                 not self.location().startswith(("/starting", "/")), 30, "an auth screen")
        wait_for(lambda: self.location() != "/starting", 30, "leaving /starting")
        if self.location() != "/login":
            log(f"not on /login after bootstrap but on {self.location()}; continuing")
            return
        # **Text entry must happen with semantics off.** With Flutter's
        # semantics tree enabled, a tapped field's focus goes to the
        # accessibility <input>, which the TextField does not read from, so
        # typed text is silently dropped. So: map the screen with semantics on,
        # reload the page (semantics off, same layout at the same viewport),
        # then tap and type into Flutter's own editing element.
        self.page.eval("window.__storm.enableSemantics()")
        self.settle(0.8)
        fields = wait_for(lambda: self.page.eval("window.__storm.inputs()"), 15, "login fields")
        submit = wait_for(lambda: self.page.eval(
            "window.__storm.find('^(Sign in|Create Account & Sign In|Continue)$')"),
            10, "the sign-in button")
        # One field shows as two <input>s; keep the labelled one per kind.
        targets = {}
        for f in sorted(fields, key=lambda f: not f["label"]):
            label = f["label"].lower()
            kind = ("password" if "pass" in label or f["type"] == "password"
                    else "username" if "user" in label else None)
            if kind and kind not in targets:
                targets[kind] = f
        self.page.call("Page.reload")
        self.wait_app()
        wait_for(lambda: self.location() == "/login", 30, "/login after reload")
        self.settle(1.0)
        for kind, f in targets.items():
            self.type_into(f["x"], f["y"], PASSWORD if kind == "password" else USERNAME)
        self.click(submit["x"], submit["y"])
        wait_for(lambda: self.location() not in ("/login", "/starting"), 30, "signing in")
        self.settle(2)
        log(f"signed in through the UI; at {self.location()}")

    # ---- shots --------------------------------------------------------

    def shoot(self, shot):
        switched = getattr(self, "current_viewport", None) != shot["viewport"]
        self.viewport(shot["viewport"])
        if switched or shot.get("fresh"):
            # A fresh load at the new size, as a device would be (flipping a
            # live page between desktop and phone sometimes stalled Chromium),
            # or with nothing left open by the shot before.
            self.page.call("Page.navigate", url=BASE + self.route(shot["route"]))
            self.wait_app()
            wait_for(lambda: self.location() not in ("/starting", "/login"), 30,
                     "the app after reload")
            self.settle(1.5)
        self.go(self.route(shot["route"]))
        for action in shot.get("actions", []):
            kind, arg = action
            if kind == "tap":
                self.tap_text(arg)
            elif kind == "go":
                self.go(self.route(arg))
            elif kind == "wait":
                time.sleep(arg)
            elif kind == "api":
                # Seeds real server state (a key, a relay) through its API.
                method, path, body = arg
                status, _ = storm_auth._call(BASE, method, self.route(path), body=body,
                                             auth=self.session)
                if status >= 300:
                    raise RuntimeError(f"seeding {method} {path} answered {status}")
        if not any(kind == "tap" for kind, _ in shot.get("actions", [])):
            # Park the pointer in a corner so no row is captured mid-hover.
            self.page.call("Input.dispatchMouseEvent", type="mouseMoved", x=1, y=1)
        self.settle(shot.get("settle", 1.5))
        png = self.page.call("Page.captureScreenshot", format="png", captureBeyondViewport=False)
        path = os.path.join(self.out, shot["name"] + ".png")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as fh:
            fh.write(base64.b64decode(png["data"]))
        log(f"{shot['name']}: {self.location()} → {os.path.relpath(path, ROOT)}")

    def debug_dump(self):
        """On failure: what the page looked like, and what it offered."""
        page = getattr(self, "page", None)
        if page is None:
            return
        try:
            png = page.call("Page.captureScreenshot", format="png")
            path = os.path.join(self.tmp, "failure.png")
            with open(path, "wb") as fh:
                fh.write(base64.b64decode(png["data"]))
            log(f"failure screenshot: {path}")
            log(f"location: {self.location()}")
            log(f"inputs: {page.eval('window.__storm && window.__storm.inputs()')}")
            labels = page.eval(
                "window.__storm && window.__storm.nodes().map(e => window.__storm.label(e))"
                ".filter(Boolean).slice(0, 40)")
            log(f"labels: {labels}")
        except Exception as e:  # the dump must never mask the real failure
            log(f"debug dump failed: {e}")

    def close(self):
        for p in reversed(self.procs):
            p.send_signal(signal.SIGTERM)
            try:
                p.wait(timeout=5)
            except subprocess.TimeoutExpired:
                p.kill()
        if not self.keep:
            shutil.rmtree(self.tmp, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", default="", help="shoot names starting with this")
    ap.add_argument("--set", default="current", help="shot set in shots.py")
    ap.add_argument("--out", default=os.path.join(HERE, ".."))
    ap.add_argument("--keep", action="store_true", help="keep the temp state dir")
    args = ap.parse_args()

    h = Harness(os.path.abspath(args.out), args.keep)
    try:
        h.start_server()
        h.claim()
        h.start_browser()
        h.sign_in()
        for shot in shots.SETS[args.set]:
            if "step" in shot:
                h.step(shot["step"])
            elif shot["name"].startswith(args.only):
                h.shoot(shot)
    except Exception:
        h.debug_dump()
        raise
    finally:
        h.close()


if __name__ == "__main__":
    main()
