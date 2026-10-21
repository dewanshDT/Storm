"""A loopback HTTP proxy between one storm-runtime and the server, for
token_sync_e2e.py.

It forwards every request unchanged and streams every answer (the host link
is SSE), and gives the suite two handles on the host's side of the wire:

- `bearer`: the host token the runtime last sent, so the suite can call the
  pull route as that host;
- `hold()` / `release()`: park the next provider-credential pull until the
  suite has changed the scope, so the server's refusal is a real one.
"""
import http.client
import http.server
import threading

HOP = {"connection", "keep-alive", "transfer-encoding", "te", "trailer", "upgrade",
       "proxy-authorization", "proxy-authenticate"}


class LinkProxy:
    def __init__(self, upstream_port):
        self.upstream_port = upstream_port
        self.bearer = None
        self.pulls = []
        self._hold = False
        self.held = threading.Event()
        self._release = threading.Event()
        proxy = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.0"

            def log_message(self, *args):
                pass

            def _forward(self):
                auth = self.headers.get("authorization", "")
                if auth.startswith("Bearer sht_"):
                    proxy.bearer = auth
                if self.path.endswith("/provider-credential"):
                    proxy.pulls.append(self.path)
                    if proxy._hold:
                        proxy.held.set()
                        proxy._release.wait(60)
                length = int(self.headers.get("content-length") or 0)
                body = self.rfile.read(length) if length else None
                headers = {k: v for k, v in self.headers.items() if k.lower() not in HOP}
                conn = http.client.HTTPConnection("127.0.0.1", proxy.upstream_port, timeout=None)
                try:
                    conn.request(self.command, self.path, body=body, headers=headers)
                    resp = conn.getresponse()
                    self.send_response(resp.status)
                    for k, v in resp.getheaders():
                        if k.lower() not in HOP:
                            self.send_header(k, v)
                    self.send_header("connection", "close")
                    self.end_headers()
                    while True:
                        chunk = resp.read1(65536)
                        if not chunk:
                            break
                        self.wfile.write(chunk)
                        self.wfile.flush()
                except (OSError, ValueError, http.client.HTTPException):
                    pass
                finally:
                    conn.close()

            do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = _forward

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.port = self.server.server_address[1]
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def hold(self):
        self.held.clear()
        self._release.clear()
        self._hold = True

    def release(self):
        self._hold = False
        self._release.set()

    def close(self):
        self.release()
        self.server.shutdown()
