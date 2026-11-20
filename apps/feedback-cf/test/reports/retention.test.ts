import { env } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import { newReportId } from "../../src/id";
import type { StoredReport } from "../../src/report";
import { putReport, recordIdempotent, reportKey, RETENTION_MS, screenshotKey } from "../../src/store";

const RETENTION_SECONDS = RETENTION_MS / 1000;

async function seedAt(ms: number, text: string, withScreenshot = false): Promise<string> {
  const id = newReportId(ms);
  const report: StoredReport = {
    id,
    schema: 1,
    install_id: "11111111-2222-3333-4444-555555555555",
    kind: "bug",
    text,
    client: {},
    bundle: {},
    received_at: new Date(ms).toISOString(),
  };
  await putReport(env, report, withScreenshot ? new Uint8Array([1, 2, 3]) : undefined);
  return id;
}

// KV expires a key itself (`expirationTtl`), so there is no cron here to
// exercise directly — these assert the TTL is actually set on every write
// that used to rely on a daily purge job.
describe("retention via KV expirationTtl (no cron — KV drops the key itself)", () => {
  it("sets an expiration roughly RETENTION_MS from now on a report", async () => {
    const before = Math.floor(Date.now() / 1000);
    const id = await seedAt(Date.now(), "fresh");

    const { keys } = await env.REPORTS.list({ prefix: reportKey(id) });
    expect(keys).toHaveLength(1);
    expect(keys[0].expiration).toBeDefined();
    const ttl = (keys[0].expiration as number) - before;
    expect(ttl).toBeGreaterThan(RETENTION_SECONDS - 60);
    expect(ttl).toBeLessThanOrEqual(RETENTION_SECONDS + 60);
  });

  it("sets the same expiration on the screenshot key", async () => {
    const before = Math.floor(Date.now() / 1000);
    const id = await seedAt(Date.now(), "with a shot", true);

    const { keys } = await env.REPORTS.list({ prefix: screenshotKey(id) });
    expect(keys).toHaveLength(1);
    const ttl = (keys[0].expiration as number) - before;
    expect(ttl).toBeGreaterThan(RETENTION_SECONDS - 60);
    expect(ttl).toBeLessThanOrEqual(RETENTION_SECONDS + 60);
  });

  it("sets an expiration on an idempotency marker too", async () => {
    const before = Math.floor(Date.now() / 1000);
    await recordIdempotent(env, "11111111-2222-3333-4444-555555555555", "r-1", "some-id");

    const { keys } = await env.REPORTS.list({ prefix: "idempotency:11111111-2222-3333-4444-555555555555/" });
    expect(keys).toHaveLength(1);
    const ttl = (keys[0].expiration as number) - before;
    expect(ttl).toBeGreaterThan(RETENTION_SECONDS - 60);
    expect(ttl).toBeLessThanOrEqual(RETENTION_SECONDS + 60);
  });
});
