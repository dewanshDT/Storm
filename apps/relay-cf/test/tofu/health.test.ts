// 116m: an uptime check an operator can point at, saying only whether the
// deployment's configuration loads and which mode it runs.
import { SELF } from "cloudflare:test";
import { expect, it } from "vitest";

it("answers GET /health with the mode and nothing else", async () => {
  const response = await SELF.fetch("https://relay.test/health");
  expect(response.status).toBe(200);
  expect(await response.json()).toEqual({ ok: true, mode: "tofu" });
});
