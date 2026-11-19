import { describe, expect, it } from "vitest";
import { listReports, postReport, reportBody, uniqueInstallId } from "../harness";

describe("report_id idempotency", () => {
  it("a resend with the same install_id and report_id returns the same id, with 200", async () => {
    const installId = uniqueInstallId();
    const first = await postReport(reportBody({ install_id: installId, report_id: "client-r-1", text: "first try" }));
    expect(first.status).toBe(201);
    const { id } = (await first.json()) as { id: string };

    const resend = await postReport(
      reportBody({ install_id: installId, report_id: "client-r-1", text: "a retried send, maybe reworded" }),
    );
    expect(resend.status).toBe(200);
    expect(((await resend.json()) as { id: string }).id).toBe(id);

    // Stored once: the original text, not whatever the retry happened to say.
    const { reports } = (await (await listReports()).json()) as { reports: Array<{ id: string; text: string }> };
    const stored = reports.filter((r) => r.id === id);
    expect(stored).toHaveLength(1);
    expect(stored[0].text).toBe("first try");
  });

  it("a replay never spends the rate limiter's budget", async () => {
    const installId = uniqueInstallId();
    // Spends 1 of the 20-per-hour budget, genuinely — the resends below must not.
    const first = await postReport(reportBody({ install_id: installId, report_id: "client-r-2" }));
    expect(first.status).toBe(201);

    for (let i = 0; i < 25; i++) {
      const resend = await postReport(reportBody({ install_id: installId, report_id: "client-r-2" }));
      expect(resend.status, `replay ${i}`).toBe(200);
    }

    // 19 more genuinely new reports exactly fill the budget the first spent 1 of.
    for (let i = 0; i < 19; i++) {
      const fresh = await postReport(reportBody({ install_id: installId, text: `fresh ${i}` }));
      expect(fresh.status, `fresh ${i}`).toBe(201);
    }
    const over = await postReport(reportBody({ install_id: installId, text: "one too many" }));
    expect(over.status).toBe(429);
  });

  it("scopes idempotency to the install_id: another install's same report_id is a new report", async () => {
    const a = uniqueInstallId();
    const b = uniqueInstallId();
    const first = await postReport(reportBody({ install_id: a, report_id: "shared-id" }));
    const second = await postReport(reportBody({ install_id: b, report_id: "shared-id" }));
    expect(first.status).toBe(201);
    expect(second.status).toBe(201);
    const firstId = ((await first.json()) as { id: string }).id;
    const secondId = ((await second.json()) as { id: string }).id;
    expect(firstId).not.toBe(secondId);
  });
});
