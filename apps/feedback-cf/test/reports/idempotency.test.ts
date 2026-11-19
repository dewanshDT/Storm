import { afterEach, describe, expect, it } from "vitest";
import { IP_RATE_LIMIT } from "../../src/index";
import { drainGlobalLimiter, listReports, postReport, reportBody, resetGlobalLimiter, uniqueInstallId } from "../harness";

describe("report_id idempotency", () => {
  // One test here deliberately drains the singleton global limiter; put it
  // back so it never leaks into a later test (this file's or another's).
  afterEach(resetGlobalLimiter);

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

  it("a replay never spends the IP limiter's budget either", async () => {
    const installId = uniqueInstallId();
    const ip = "203.0.113.30";
    const first = await postReport(reportBody({ install_id: installId, report_id: "client-r-ip" }), {
      "cf-connecting-ip": ip,
    });
    expect(first.status).toBe(201);

    // Replaying well past IP_RATE_LIMIT proves the replay path never reaches
    // the IP limiter at all, not just that it happens to stay under it.
    for (let i = 0; i < IP_RATE_LIMIT + 10; i++) {
      const resend = await postReport(reportBody({ install_id: installId, report_id: "client-r-ip" }), {
        "cf-connecting-ip": ip,
      });
      expect(resend.status, `replay ${i}`).toBe(200);
    }
  });

  it("a replay is exempt from the global cap too", async () => {
    const installId = uniqueInstallId();
    const first = await postReport(reportBody({ install_id: installId, report_id: "client-r-global" }));
    expect(first.status).toBe(201);

    await drainGlobalLimiter();

    const resend = await postReport(reportBody({ install_id: installId, report_id: "client-r-global" }));
    expect(resend.status).toBe(200);
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
