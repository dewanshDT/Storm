"""A minimal Chrome DevTools Protocol client, standard library only.

The acceptance harness drives a real headless Chromium against the real web
client. Python's standard library has no WebSocket client, and adding one to
the repo for a docs harness would be a dependency nothing else needs, so this
file speaks just enough RFC 6455 for CDP: one text frame per message, client
frames masked, server frames unmasked, fragmentation and ping handled.
"""

import base64
import json
import os
import socket
import struct
import urllib.parse
import urllib.request


class WebSocket:
    def __init__(self, url, timeout=60):
        u = urllib.parse.urlparse(url)
        self.sock = socket.create_connection((u.hostname, u.port), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        path = u.path + (f"?{u.query}" if u.query else "")
        self.sock.sendall(
            (
                f"GET {path} HTTP/1.1\r\nHost: {u.hostname}:{u.port}\r\n"
                "Upgrade: websocket\r\nConnection: Upgrade\r\n"
                f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
            ).encode()
        )
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = self.sock.recv(1)
            if not chunk:
                raise ConnectionError("websocket handshake closed")
            head += chunk
        if b" 101 " not in head.split(b"\r\n", 1)[0]:
            raise ConnectionError(f"websocket handshake refused: {head!r}")

    def _recv_exact(self, n):
        buf = b""
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                raise ConnectionError("websocket closed")
            buf += chunk
        return buf

    def send(self, text):
        payload = text.encode()
        header = bytearray([0x81])  # FIN + text
        n = len(payload)
        if n < 126:
            header.append(0x80 | n)
        elif n < 1 << 16:
            header.append(0x80 | 126)
            header += struct.pack("!H", n)
        else:
            header.append(0x80 | 127)
            header += struct.pack("!Q", n)
        mask = os.urandom(4)
        header += mask
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.sock.sendall(bytes(header) + masked)

    def recv(self):
        message = b""
        while True:
            b1, b2 = self._recv_exact(2)
            fin, opcode = b1 & 0x80, b1 & 0x0F
            n = b2 & 0x7F
            if n == 126:
                n = struct.unpack("!H", self._recv_exact(2))[0]
            elif n == 127:
                n = struct.unpack("!Q", self._recv_exact(8))[0]
            payload = self._recv_exact(n)
            if opcode == 0x9:  # ping → pong
                self.sock.sendall(bytes([0x8A, 0x80 | len(payload)]) + b"\0\0\0\0" + payload)
                continue
            if opcode == 0x8:
                raise ConnectionError("websocket closed by peer")
            message += payload
            if fin:
                return message.decode()


class Page:
    """One CDP target. `call` waits for the matching response and drops events."""

    def __init__(self, debug_port):
        targets = json.load(urllib.request.urlopen(f"http://127.0.0.1:{debug_port}/json"))
        page = next(t for t in targets if t["type"] == "page")
        self.ws = WebSocket(page["webSocketDebuggerUrl"])
        self.next_id = 0

    def call(self, method, **params):
        self.next_id += 1
        mid = self.next_id
        self.ws.send(json.dumps({"id": mid, "method": method, "params": params}))
        while True:
            msg = json.loads(self.ws.recv())
            if msg.get("id") == mid:
                if "error" in msg:
                    raise RuntimeError(f"{method}: {msg['error']}")
                return msg.get("result", {})

    def eval(self, expression):
        r = self.call(
            "Runtime.evaluate", expression=expression, returnByValue=True, awaitPromise=True
        )
        if "exceptionDetails" in r:
            raise RuntimeError(f"eval failed: {r['exceptionDetails']}")
        return r.get("result", {}).get("value")
