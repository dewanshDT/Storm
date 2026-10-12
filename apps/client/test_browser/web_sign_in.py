"""The web sign-in, in a real Firefox, against a real storm-server (decision 90).

Firefox-engine browsers (Zen) stayed on the login spinner while the server
logged every sign-in as a success: something submitted Flutter's hidden
autofill <form> natively, the browser POSTed the page, and the app reloaded
mid sign-in. Each scenario here signs in from a fresh profile and must land in
the app with its tokens stored, after one page load.

usage: web_sign_in.py STORM_SERVER_BIN WEB_BUILD_DIR
  BROWSER   Firefox-engine binary (default: firefox, else Zen's zen-bin)
  HOST      address to serve on; a non-loopback one makes the origin
            insecure, as on a LAN (default: this machine's first one)
"""
import asyncio, base64, json, os, shutil, socket, subprocess, sys, tempfile, time

import websockets

PASSWORD = "correct horse battery staple"
RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append(ok)
    print(f"  {'PASS' if ok else 'FAIL'}  {name}" + ("" if ok else f"   {detail}"), flush=True)


def browser_bin():
    if os.environ.get("BROWSER"):
        return os.environ["BROWSER"]
    for b in ("firefox", "/opt/zen-browser-bin/zen-bin", "zen-browser"):
        if shutil.which(b):
            return shutil.which(b)
    sys.exit("no Firefox-engine browser: set BROWSER")


def lan_address():
    if os.environ.get("HOST"):
        return os.environ["HOST"]
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(("10.255.255.255", 1))
        return s.getsockname()[0]
    except OSError:
        return "127.0.0.1"
    finally:
        s.close()


def free_port():
    s = socket.socket()
    s.bind(("", 0))
    port = s.getsockname()[1]
    s.close()
    return port


class Browser:
    """One headless browser with a fresh profile, over WebDriver BiDi."""

    def __init__(self, preload=None):
        self.preload, self.n, self.pending, self.events = preload, 0, {}, []

    async def __aenter__(self):
        self.profile = tempfile.mkdtemp(prefix="storm-ff-")
        with open(os.path.join(self.profile, "user.js"), "w") as f:
            f.write('user_pref("remote.active-protocols", 1);\n'
                    'user_pref("browser.shell.checkDefaultBrowser", false);\n'
                    'user_pref("datareporting.policy.dataSubmissionEnabled", false);\n')
        port = free_port()
        self.proc = subprocess.Popen(
            [browser_bin(), "--headless", "--no-remote", "--profile", self.profile,
             f"--remote-debugging-port={port}", "--window-size=1280,900"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(150):
            try:
                self.ws = await websockets.connect(f"ws://127.0.0.1:{port}/session", max_size=50_000_000)
                break
            except OSError:
                await asyncio.sleep(0.2)
        else:
            raise RuntimeError("the browser never opened its BiDi port")
        self.pump = asyncio.create_task(self._pump())
        await self.cmd("session.new", capabilities={})
        await self.cmd("session.subscribe", events=["browsingContext.load", "log.entryAdded"])
        self.ctx = (await self.cmd("browsingContext.getTree"))["contexts"][0]["context"]
        if self.preload:
            await self.cmd("script.addPreloadScript", functionDeclaration=f"() => {{ {self.preload} }}")
        return self

    async def __aexit__(self, *exc):
        self.pump.cancel()
        try:
            await self.ws.close()
        except Exception:
            pass
        self.proc.terminate()
        try:
            self.proc.wait(5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        shutil.rmtree(self.profile, ignore_errors=True)

    async def _pump(self):
        async for raw in self.ws:
            m = json.loads(raw)
            if m.get("id") in self.pending:
                self.pending.pop(m["id"]).set_result(m)
            elif m.get("type") == "event":
                self.events.append(m)

    async def cmd(self, method, **params):
        self.n += 1
        fut = asyncio.get_running_loop().create_future()
        self.pending[self.n] = fut
        await self.ws.send(json.dumps({"id": self.n, "method": method, "params": params}))
        m = await asyncio.wait_for(fut, 60)
        if m.get("type") == "error":
            raise RuntimeError(f"{method}: {m.get('error')}: {m.get('message')}")
        return m["result"]

    async def js(self, expr):
        r = await self.cmd("script.evaluate", expression=expr, target={"context": self.ctx},
                           awaitPromise=True)
        return r.get("result", {}).get("value")

    def loads(self):
        return sum(1 for e in self.events if e["method"] == "browsingContext.load")

    def console(self):
        return [e["params"].get("text", "") for e in self.events if e["method"] == "log.entryAdded"]

    async def open(self, url):
        await self.cmd("browsingContext.navigate", context=self.ctx, url=url, wait="complete")

    async def key(self, value):
        await self.cmd("input.performActions", context=self.ctx, actions=[
            {"type": "key", "id": "kb", "actions": [
                {"type": "keyDown", "value": value}, {"type": "keyUp", "value": value}]}])

    async def type_password(self):
        """Flutter puts an <input type=password> in the DOM once the field has
        focus. Autofocus normally gives it that; a slow runner can miss it after
        a reload, so Tab moves focus there, as a person would."""
        for i in range(150):
            if await self.js("!!document.querySelector('input[type=password]')"):
                break
            if i and i % 15 == 0:
                await self.key("\ue004")
            await asyncio.sleep(0.2)
        else:
            raise RuntimeError(f"the login field never appeared at {await self.js('location.pathname')}")
        acts = []
        for ch in PASSWORD + "":
            acts += [{"type": "keyDown", "value": ch}, {"type": "keyUp", "value": ch}]
        await self.cmd("input.performActions", context=self.ctx,
                       actions=[{"type": "key", "id": "kb", "actions": acts}])

    async def signed_in(self, timeout=15):
        """In the app with a stored access token, or what it was doing instead."""
        deadline = time.time() + timeout
        state = None
        while time.time() < deadline:
            try:
                state = json.loads(await self.js(
                    "JSON.stringify({path: location.pathname,"
                    " token: !!localStorage.getItem('flutter.storm.accessToken')})"))
            except Exception as e:
                state = {"error": str(e)}
            if state.get("path", "").startswith(("/v/", "/notes")) and state.get("token"):
                return True, state
            await asyncio.sleep(0.25)
        return False, state


# What a password manager or the browser might do to the login form. Each runs
# in the page before the app, and acts on the form holding the password field.
FORM_OF_PASSWORD = """
  const formOf = (e) => { const i = e.composedPath()[0]; return i && i.form; };
"""
SCENARIOS = {
    "Enter, nothing else": None,
    "a native form.submit() just after Enter (a password manager capturing the login)": FORM_OF_PASSWORD + """
  document.addEventListener('keydown', (e) => {
    const f = e.key === 'Enter' && formOf(e);
    if (f) setTimeout(() => f.submit(), 50);
  }, true);""",
    "a native form.submit() as the sign-in request goes out": FORM_OF_PASSWORD + """
  let form = null;
  document.addEventListener('keydown', (e) => { if (e.key === 'Enter') form = formOf(e) || form; }, true);
  const fire = (url) => { if (form && String(url).includes('/v1/auth/login')) form.submit(); };
  const open = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function (m, url) { this.__url = url; return open.apply(this, arguments); };
  const send = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.send = function () { const r = send.apply(this, arguments); fire(this.__url); return r; };
  const fetch0 = window.fetch;
  window.fetch = function (input) { const p = fetch0.apply(this, arguments); fire(input && input.url || input); return p; };""",
    "form.requestSubmit() just after Enter": FORM_OF_PASSWORD + """
  document.addEventListener('keydown', (e) => {
    const f = e.key === 'Enter' && formOf(e);
    if (f) setTimeout(() => f.requestSubmit(), 50);
  }, true);""",
    # What the operator's Zen profile held: builds before 2026-08-19 stored an
    # empty server address on the web, and later builds kept it. Sign-in then
    # succeeded on the server, relative to the page, while the app — with no
    # address — never counted itself configured and stayed on /login.
    "a browser that stored an empty server address (an older build's)": """
  localStorage.setItem('flutter.storm.baseUrl', '""');""",
    "an inline autofill menu that keeps itself last in <body>": """
  let button = null, list = null;
  document.addEventListener('keydown', () => {
    if (button || !document.querySelector('input[type=password]')) return;
    button = document.createElement('pm-button'); list = document.createElement('pm-list');
    document.body.append(button, list);
    new MutationObserver(() => {
      const last = document.body.lastElementChild;
      if (last === list) return;
      if (last === button) { document.body.appendChild(list); return; }
      try { document.body.insertBefore(last, button); } catch (e) { console.error(String(e)); }
    }).observe(document.body, { childList: true });
  }, true);""",
}


async def sign_in(base, name, preload):
    async with Browser(preload) as b:
        await b.open(base + "/")
        await b.type_password()
        ok, state = await b.signed_in()
        loads = b.loads()
        check(name, ok and loads == 1,
              f"state={state} page loads={loads} console={[c for c in b.console() if 'secure storage' not in c][-5:]}")


async def sign_in_again(base):
    """A paired browser whose session is gone, which is the /login the issue started on."""
    async with Browser() as b:
        await b.open(base + "/")
        await b.type_password()
        ok, state = await b.signed_in()
        check("a first sign-in, before signing in again", ok, f"state={state}")
        await b.js("['flutter.storm.accessToken', 'flutter.storm.refreshToken']"
                   ".forEach(k => localStorage.removeItem(k)); true")
        await b.open(base + "/")
        await b.type_password()
        ok, state = await b.signed_in()
        check("a paired browser signs in again", ok, f"state={state}")


async def main(server_bin, web_dir):
    work = tempfile.mkdtemp(prefix="storm-web-sign-in-")
    os.makedirs(os.path.join(work, "vaults", "primary"))
    with open(os.path.join(work, "vaults", "primary", "Seed.md"), "w") as f:
        f.write("# Seed\n")
    state = os.path.join(work, "state")
    subprocess.run([server_bin, "passwd", "--state", state, "--password-stdin"],
                   input=PASSWORD.encode(), check=True, capture_output=True)
    host, port = lan_address(), free_port()
    base = f"http://{host}:{port}"
    log = open(os.path.join(work, "server.log"), "w")
    server = subprocess.Popen(
        [server_bin, "serve", "--vault-root", os.path.join(work, "vaults"), "--state", state,
         "--host", host, "--port", str(port), "--web", web_dir],
        stdout=log, stderr=subprocess.STDOUT)
    try:
        import urllib.request
        for _ in range(120):
            try:
                urllib.request.urlopen(base + "/v1/health", timeout=1)
                break
            except OSError:
                time.sleep(0.25)
        print(f"\n=== the web sign-in in {os.path.basename(browser_bin())}, at {base} ===")
        for name, preload in SCENARIOS.items():
            await sign_in(base, name, preload)
        await sign_in_again(base)
    finally:
        server.terminate()
        server.wait(10)
        if all(RESULTS):
            shutil.rmtree(work, ignore_errors=True)
        else:
            print(f"  server log: {os.path.join(work, 'server.log')}")
    passed = sum(RESULTS)
    print(f"  {passed} passed, {len(RESULTS) - passed} failed")
    sys.exit(0 if all(RESULTS) else 1)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    asyncio.run(main(sys.argv[1], sys.argv[2]))
