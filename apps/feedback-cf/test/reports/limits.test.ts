import { env, runInDurableObject } from "cloudflare:test";
import { afterEach, describe, expect, it } from "vitest";
import { GLOBAL_DAILY_LIMIT, GLOBAL_LIMITER_NAME, GLOBAL_WINDOW_MS, IP_RATE_LIMIT } from "../../src/index";
import { drainGlobalLimiter, postReport, reportBody, resetGlobalLimiter, uniqueInstallId } from "../harness";

// TEST-NET-3 (RFC 5737): never a real address.
const IP_A = "203.0.113.10";
const IP_B = "203.0.113.20";

const FIVE_MIB = 5 * 1024 * 1024;

describe("413: the 5 MB body cap", () => {
  it("refuses a Content-Length over the cap before reading the body", async () => {
    const body = JSON.stringify(reportBody());
    const res = await postReport(body, { "content-length": String(FIVE_MIB + 1) });
    expect(res.status).toBe(413);
  });

  it("refuses a body that is actually over the cap", async () => {
    // Legal per-field sizes (text stays under 20k); bundle is what's over 5 MB.
    const body = reportBody({ bundle: { pad: "x".repeat(FIVE_MIB + 1024) } });
    const res = await postReport(body);
    expect(res.status).toBe(413);
  });

  it("accepts an ordinary body well under the cap", async () => {
    const res = await postReport(reportBody({ install_id: uniqueInstallId() }));
    expect(res.status).toBe(201);
  });
});

describe("429: 20 reports per rolling hour per install_id", () => {
  it("allows 20 and refuses the 21st, with a Retry-After", async () => {
    const installId = uniqueInstallId();
    for (let i = 0; i < 20; i++) {
      const res = await postReport(reportBody({ install_id: installId, text: `report ${i}` }));
      expect(res.status, `request ${i}`).toBe(201);
    }
    const res = await postReport(reportBody({ install_id: installId, text: "report 20" }));
    expect(res.status).toBe(429);
    const retryAfter = Number(res.headers.get("retry-after"));
    expect(Number.isFinite(retryAfter)).toBe(true);
    expect(retryAfter).toBeGreaterThan(0);
  });

  it("limits per install_id, not globally", async () => {
    const a = uniqueInstallId();
    const b = uniqueInstallId();
    for (let i = 0; i < 20; i++) expect((await postReport(reportBody({ install_id: a, text: `a${i}` }))).status).toBe(201);
    expect((await postReport(reportBody({ install_id: a, text: "a20" }))).status).toBe(429);
    expect((await postReport(reportBody({ install_id: b, text: "b0" }))).status).toBe(201);
  });
});

// Backstop for an abuser rotating install_id to dodge the limit above.
describe(`429: ${IP_RATE_LIMIT} reports per rolling hour per IP`, () => {
  it(`allows ${IP_RATE_LIMIT} and refuses the next, even with a brand-new install_id, with a Retry-After`, async () => {
    for (let i = 0; i < IP_RATE_LIMIT; i++) {
      const res = await postReport(reportBody({ install_id: uniqueInstallId(), text: `ip ${i}` }), {
        "cf-connecting-ip": IP_A,
      });
      expect(res.status, `request ${i}`).toBe(201);
    }
    const res = await postReport(reportBody({ install_id: uniqueInstallId(), text: "over the ip limit" }), {
      "cf-connecting-ip": IP_A,
    });
    expect(res.status).toBe(429);
    const retryAfter = Number(res.headers.get("retry-after"));
    expect(Number.isFinite(retryAfter)).toBe(true);
    expect(retryAfter).toBeGreaterThan(0);
  });

  it("limits per IP, not globally: a different IP has its own budget", async () => {
    const res = await postReport(reportBody({ install_id: uniqueInstallId(), text: "fresh ip" }), {
      "cf-connecting-ip": IP_B,
    });
    expect(res.status).toBe(201);
  });

  it("skips the IP limiter entirely when CF-Connecting-IP is absent (local dev, direct tests)", async () => {
    for (let i = 0; i < IP_RATE_LIMIT + 5; i++) {
      const res = await postReport(reportBody({ install_id: uniqueInstallId(), text: `no ip ${i}` }));
      expect(res.status, `request ${i}`).toBe(201);
    }
  });
});

describe(`503: a ${GLOBAL_DAILY_LIMIT}/day global cap on stored reports`, () => {
  afterEach(resetGlobalLimiter);

  it("refuses with a Retry-After once the shared daily budget is spent", async () => {
    await drainGlobalLimiter();

    const res = await postReport(reportBody({ install_id: uniqueInstallId(), text: "past the global cap" }));
    expect(res.status).toBe(503);
    const retryAfter = Number(res.headers.get("retry-after"));
    expect(Number.isFinite(retryAfter)).toBe(true);
    expect(retryAfter).toBeGreaterThan(0);
  });

  it("refills over the window like any other token bucket", async () => {
    const stub = env.RATE_LIMITER.get(env.RATE_LIMITER.idFromName(GLOBAL_LIMITER_NAME));
    await runInDurableObject(stub, async (_instance, state) => {
      await state.storage.put("bucket", { tokens: 0, last: Date.now() - GLOBAL_WINDOW_MS });
    });

    const res = await postReport(reportBody({ install_id: uniqueInstallId(), text: "window refilled" }));
    expect(res.status).toBe(201);
  });
});
