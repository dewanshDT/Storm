// The runtime behaviour trySend exists for: a send throws once a socket is
// closing, whichever side closed it, and messages that arrived before a close
// are still delivered after it.
import { describe, expect, it } from "vitest";
import { trySend } from "../../src/send";

function pair(): [WebSocket, WebSocket] {
  const [a, b] = Object.values(new WebSocketPair());
  a.accept();
  b.accept();
  return [a, b];
}

describe("trySend", () => {
  it("refuses a socket this side closed, where send throws", () => {
    const [a] = pair();
    a.close(1000);
    expect(() => a.send("x")).toThrow(/after close/);
    expect(trySend(a, "x")).toBe(false);
  });

  it("refuses a socket the other side closed, where send throws", async () => {
    const [a, b] = pair();
    const closed = new Promise((r) => a.addEventListener("close", r));
    b.close(1000);
    await closed;
    expect(() => a.send("x")).toThrow(/after close/);
    expect(trySend(a, new ArrayBuffer(1))).toBe(false);
  });

  it("is reached by a message delivered after its own socket closed", async () => {
    const [a, b] = pair();
    const results: boolean[] = [];
    a.addEventListener("message", () => {
      results.push(trySend(a, "pong"));
    });
    b.send("ping");
    a.close(1000);
    await new Promise((r) => setTimeout(r, 50));
    expect(results).toEqual([false]);
  });

  it("sends on an open socket", async () => {
    const [a, b] = pair();
    const got = new Promise((r) => b.addEventListener("message", (e) => r(e.data)));
    expect(trySend(a, "hello")).toBe(true);
    expect(await got).toBe("hello");
  });
});
