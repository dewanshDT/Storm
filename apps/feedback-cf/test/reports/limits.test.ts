import { describe, expect, it } from "vitest";
import { postReport, reportBody, uniqueInstallId } from "../harness";

const FIVE_MIB = 5 * 1024 * 1024;

describe("413: the 5 MB body cap", () => {
  it("refuses a Content-Length over the cap before reading the body", async () => {
    const body = JSON.stringify(reportBody());
    const res = await postReport(body, { "content-length": String(FIVE_MIB + 1) });
    expect(res.status).toBe(413);
  });

  it("refuses a body that is actually over the cap", async () => {
    // Legal per-field sizes (text stays under 20k); the bundle is what's over
    // 5 MB, since 162d is permissive about bundle contents.
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
    // A different install_id has its own untouched budget.
    expect((await postReport(reportBody({ install_id: b, text: "b0" }))).status).toBe(201);
  });
});
