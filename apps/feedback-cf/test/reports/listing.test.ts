import { env } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import { newReportId } from "../../src/id";
import type { StoredReport } from "../../src/report";
import { listReportIds, putReport } from "../../src/store";
import { listReports, uniqueInstallId } from "../harness";

// Writes straight to the store with a chosen timestamp — two requests sent
// over HTTP can land in the same millisecond, which would make an order
// assertion flaky for no reason the API itself cares about.
async function seed(ms: number, text: string): Promise<string> {
  const id = newReportId(ms);
  const report: StoredReport = {
    id,
    schema: 1,
    install_id: uniqueInstallId(),
    kind: "bug",
    text,
    client: {},
    bundle: {},
    received_at: new Date(ms).toISOString(),
  };
  await putReport(env, report, undefined);
  return id;
}

describe("listing order", () => {
  it("is oldest first, because ids are time-prefixed and KV lists a prefix ascending", async () => {
    const base = Date.UTC(2026, 0, 1, 0, 0, 0);
    const oldest = await seed(base, "oldest");
    const middle = await seed(base + 1000, "middle");
    const newest = await seed(base + 2000, "newest");

    const listed = await listReportIds(env, 50);
    const ids = listed.map((entry) => entry.id);
    const positions = [oldest, middle, newest].map((id) => ids.indexOf(id));
    expect(positions).toEqual([...positions].sort((a, b) => a - b));
    expect(positions.every((p) => p >= 0)).toBe(true);
  });

  it("the HTTP endpoint preserves that order", async () => {
    const base = Date.UTC(2026, 0, 2, 0, 0, 0);
    const oldest = await seed(base, "oldest");
    const newest = await seed(base + 60_000, "newest");

    const { reports } = (await (await listReports()).json()) as { reports: Array<{ id: string }> };
    const oldestPos = reports.findIndex((r) => r.id === oldest);
    const newestPos = reports.findIndex((r) => r.id === newest);
    expect(oldestPos).toBeGreaterThanOrEqual(0);
    expect(newestPos).toBeGreaterThan(oldestPos);
  });
});
