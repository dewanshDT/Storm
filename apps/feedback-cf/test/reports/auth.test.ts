import { describe, expect, it } from "vitest";
import { deleteReportRequest, fetchScreenshot, listReports, postAndExpectStored } from "../harness";

async function isListed(id: string): Promise<boolean> {
  const { reports } = (await (await listReports()).json()) as { reports: Array<{ id: string }> };
  return reports.some((r) => r.id === id);
}

describe("Authorization on the sync endpoints", () => {
  it("401s GET /v1/reports with no header, the wrong scheme or the wrong token", async () => {
    expect((await listReports("", {})).status).toBe(401);
    expect((await listReports("", { authorization: "Basic test-sync-token" })).status).toBe(401);
    expect((await listReports("", { authorization: "Bearer not-it" })).status).toBe(401);
  });

  it("401s the screenshot and DELETE routes the same way", async () => {
    const { id } = await postAndExpectStored();
    expect((await fetchScreenshot(id, {})).status).toBe(401);
    expect((await fetchScreenshot(id, { authorization: "Bearer not-it" })).status).toBe(401);
    expect((await deleteReportRequest(id, {})).status).toBe(401);
    expect((await deleteReportRequest(id, { authorization: "Bearer not-it" })).status).toBe(401);
  });

  it("a 401'd DELETE never touches the data a wrong token asked for", async () => {
    const { id } = await postAndExpectStored();
    const refused = await deleteReportRequest(id, { authorization: "Bearer not-it" });
    expect(refused.status).toBe(401);
    expect(await isListed(id)).toBe(true);
    expect((await deleteReportRequest(id)).status).toBe(204);
    expect(await isListed(id)).toBe(false);
  });
});
