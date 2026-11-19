import { describe, expect, it } from "vitest";
import {
  deleteReportRequest,
  fetchScreenshot,
  listReports,
  postAndExpectStored,
  postReport,
  reportBody,
  TINY_PNG_B64,
  uniqueInstallId,
} from "../harness";

describe("POST /v1/reports", () => {
  it("stores a report and returns a 201 with its id", async () => {
    const { id } = await postAndExpectStored();
    expect(id.length).toBe(26);
  });

  it("takes no Authorization header at all", async () => {
    const res = await postReport(reportBody());
    expect(res.status).toBe(201);
  });
});

describe("GET /v1/reports, the screenshot and DELETE", () => {
  it("lists a stored report's full contents plus has_screenshot", async () => {
    const installId = uniqueInstallId();
    const plain = await postAndExpectStored({ install_id: installId, text: "first" });
    const withShot = await postAndExpectStored({ install_id: installId, text: "second", screenshot_png_b64: TINY_PNG_B64 });

    const res = await listReports();
    expect(res.status).toBe(200);
    const { reports } = (await res.json()) as { reports: Array<Record<string, unknown>> };
    const mine = reports.filter((r) => r.install_id === installId);
    expect(mine.map((r) => r.id).sort()).toEqual([plain.id, withShot.id].sort());

    const plainRow = mine.find((r) => r.id === plain.id)!;
    const shotRow = mine.find((r) => r.id === withShot.id)!;
    expect(plainRow.has_screenshot).toBe(false);
    expect(shotRow.has_screenshot).toBe(true);
    expect(shotRow.text).toBe("second");
    expect(shotRow.kind).toBe("bug");
    expect(typeof shotRow.received_at).toBe("string");
  });

  it("streams the screenshot bytes with an image/png content-type", async () => {
    const { id } = await postAndExpectStored({ screenshot_png_b64: TINY_PNG_B64 });
    const res = await fetchScreenshot(id);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toBe("image/png");
    const bytes = new Uint8Array(await res.arrayBuffer());
    // The PNG signature.
    expect([...bytes.slice(0, 8)]).toEqual([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  });

  it("404s a screenshot fetch for a report with none", async () => {
    const { id } = await postAndExpectStored();
    expect((await fetchScreenshot(id)).status).toBe(404);
  });

  it("deletes both objects, after which nothing about the id answers", async () => {
    const { id } = await postAndExpectStored({ screenshot_png_b64: TINY_PNG_B64 });
    expect((await deleteReportRequest(id)).status).toBe(204);
    expect((await deleteReportRequest(id)).status).toBe(404);
    expect((await fetchScreenshot(id)).status).toBe(404);

    const { reports } = (await (await listReports()).json()) as { reports: Array<{ id: string }> };
    expect(reports.some((r) => r.id === id)).toBe(false);
  });

  it("limit= caps the page size", async () => {
    const installId = uniqueInstallId();
    for (let i = 0; i < 3; i++) await postAndExpectStored({ install_id: installId, text: `n${i}` });
    const res = await listReports("?limit=2");
    expect(res.status).toBe(200);
    const { reports } = (await res.json()) as { reports: unknown[] };
    expect(reports.length).toBeLessThanOrEqual(2);
  });

  it("400s a non-numeric or non-positive limit", async () => {
    expect((await listReports("?limit=abc")).status).toBe(400);
    expect((await listReports("?limit=0")).status).toBe(400);
    expect((await listReports("?limit=-1")).status).toBe(400);
  });
});
