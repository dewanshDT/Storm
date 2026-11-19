import { SELF } from "cloudflare:test";
import { expect } from "vitest";
import type { ReportKind } from "../src/report";

// Matches vitest.config.ts's miniflare bindings.
export const SYNC_TOKEN = "test-sync-token";

// A valid 1x1 PNG, base64 (decision 162d's screenshot_png_b64).
export const TINY_PNG_B64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAAAAAA6fptVAAAACklEQVR4nGMAAQAABQABDQottAAAAABJRU5ErkJggg==";

export function uniqueInstallId(): string {
  return crypto.randomUUID();
}

export interface ReportOverrides {
  schema?: unknown;
  install_id?: unknown;
  report_id?: unknown;
  kind?: unknown;
  text?: unknown;
  email?: unknown;
  client?: unknown;
  bundle?: unknown;
  screenshot_png_b64?: unknown;
}

export function reportBody(overrides: ReportOverrides = {}): Record<string, unknown> {
  const body: Record<string, unknown> = {
    schema: 1,
    install_id: uniqueInstallId(),
    kind: "bug" as ReportKind,
    text: "the sync button did nothing",
    client: { version: "0.18.0", platform: "linux-x64 · desktop" },
    bundle: { versions: { server: "0.18.0" } },
  };
  for (const [key, value] of Object.entries(overrides)) {
    if (value === undefined) delete body[key];
    else body[key] = value;
  }
  return body;
}

export function authHeaders(token: string = SYNC_TOKEN): Record<string, string> {
  return { authorization: `Bearer ${token}` };
}

export async function postReport(body: unknown, extraHeaders: Record<string, string> = {}): Promise<Response> {
  const init: RequestInit = {
    method: "POST",
    headers: { "content-type": "application/json", ...extraHeaders },
    body: typeof body === "string" ? body : JSON.stringify(body),
  };
  return SELF.fetch("https://feedback.test/v1/reports", init);
}

export async function postAndExpectStored(
  overrides: ReportOverrides = {},
): Promise<{ id: string; body: Record<string, unknown> }> {
  const body = reportBody(overrides);
  const res = await postReport(body);
  expect(res.status, await res.clone().text()).toBe(201);
  const parsed = (await res.json()) as { id: string };
  expect(typeof parsed.id).toBe("string");
  return { id: parsed.id, body };
}

export async function listReports(query = "", headers: Record<string, string> = authHeaders()): Promise<Response> {
  return SELF.fetch(`https://feedback.test/v1/reports${query}`, { headers });
}

export async function fetchScreenshot(id: string, headers: Record<string, string> = authHeaders()): Promise<Response> {
  return SELF.fetch(`https://feedback.test/v1/reports/${encodeURIComponent(id)}/screenshot`, { headers });
}

export async function deleteReportRequest(id: string, headers: Record<string, string> = authHeaders()): Promise<Response> {
  return SELF.fetch(`https://feedback.test/v1/reports/${encodeURIComponent(id)}`, { method: "DELETE", headers });
}
