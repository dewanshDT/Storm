import { createExecutionContext, createScheduledController, env, waitOnExecutionContext } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import worker from "../../src/index";
import { newReportId } from "../../src/id";
import type { StoredReport } from "../../src/report";
import { putReport, reportKey, RETENTION_MS, screenshotKey } from "../../src/store";
import { postReport, reportBody, uniqueInstallId } from "../harness";

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

describe("scheduled retention", () => {
  it("purges a report and its screenshot past 30 days, keeps a recent one", async () => {
    const now = Date.UTC(2026, 11, 1);
    const expired = await seedAt(now - RETENTION_MS - 60_000, "too old", true);
    const kept = await seedAt(now - RETENTION_MS + 60_000, "still within the window");

    const ctx = createExecutionContext();
    await worker.scheduled(createScheduledController({ scheduledTime: now }), env, ctx);
    await waitOnExecutionContext(ctx);

    expect(await env.FEEDBACK.head(reportKey(expired))).toBeNull();
    expect(await env.FEEDBACK.head(screenshotKey(expired))).toBeNull();
    expect(await env.FEEDBACK.head(reportKey(kept))).not.toBeNull();
  });

  it("leaves a report with no decodable time prefix alone rather than guessing", async () => {
    await env.FEEDBACK.put("reports/not-a-real-id-xxxxxxxxxx/report.json", JSON.stringify({ weird: true }));

    const ctx = createExecutionContext();
    await worker.scheduled(createScheduledController(), env, ctx);
    await waitOnExecutionContext(ctx);

    expect(await env.FEEDBACK.head("reports/not-a-real-id-xxxxxxxxxx/report.json")).not.toBeNull();
  });

  it("also purges idempotency markers past the same window, so a late resend starts fresh", async () => {
    const installId = uniqueInstallId();
    const first = await postReport(reportBody({ install_id: installId, report_id: "late-resend" }));
    expect(first.status).toBe(201);
    const firstId = ((await first.json()) as { id: string }).id;

    const future = Date.now() + RETENTION_MS + 24 * 60 * 60 * 1000;
    const ctx = createExecutionContext();
    await worker.scheduled(createScheduledController({ scheduledTime: future }), env, ctx);
    await waitOnExecutionContext(ctx);

    expect(await env.FEEDBACK.head(reportKey(firstId))).toBeNull();

    const resend = await postReport(reportBody({ install_id: installId, report_id: "late-resend" }));
    expect(resend.status).toBe(201);
    expect(((await resend.json()) as { id: string }).id).not.toBe(firstId);
  });

  it("leaves a recent idempotency marker alone — nothing to purge yet", async () => {
    const installId = uniqueInstallId();
    const first = await postReport(reportBody({ install_id: installId, report_id: "recent" }));
    const firstId = ((await first.json()) as { id: string }).id;

    const ctx = createExecutionContext();
    await worker.scheduled(createScheduledController(), env, ctx);
    await waitOnExecutionContext(ctx);

    const resend = await postReport(reportBody({ install_id: installId, report_id: "recent" }));
    expect(resend.status).toBe(200);
    expect(((await resend.json()) as { id: string }).id).toBe(firstId);
  });
});
