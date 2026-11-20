import type { Env } from "./env";
import { isValidReportId } from "./id";
import type { StoredReport } from "./report";

const REPORT_PREFIX = "report:";
const SCREENSHOT_PREFIX = "shot:";
const IDEMPOTENCY_PREFIX = "idempotency:";

export function reportKey(id: string): string {
  return `${REPORT_PREFIX}${id}`;
}

export function screenshotKey(id: string): string {
  return `${SCREENSHOT_PREFIX}${id}`;
}

// Unsynced reports are dropped after 30 days; the sync service deletes a
// report as soon as it lands in the vault, so anything still here past the
// cutoff was never synced. KV's own `expirationTtl` (set on every write
// below) does the dropping — there is no cron job (decision 162, R2->KV).
export const RETENTION_MS = 30 * 24 * 60 * 60 * 1000;
const RETENTION_SECONDS = RETENTION_MS / 1000;

// What the list endpoint needs about a report without fetching its value.
// `has_screenshot` is the one actually read back out (by `listReportIds`,
// from the key's own `list()` page); the rest is for an operator poking at
// `wrangler kv key list` without a GET per key.
interface ReportMetadata {
  created_at: string;
  size: number;
  kind: string;
  has_screenshot: boolean;
}

export async function putReport(env: Env, report: StoredReport, screenshot: Uint8Array | undefined): Promise<void> {
  const body = JSON.stringify(report);
  const metadata: ReportMetadata = {
    created_at: report.received_at,
    size: body.length,
    kind: report.kind,
    has_screenshot: screenshot !== undefined,
  };
  await env.REPORTS.put(reportKey(report.id), body, { expirationTtl: RETENTION_SECONDS, metadata });
  if (screenshot) {
    await env.REPORTS.put(screenshotKey(report.id), screenshot, {
      expirationTtl: RETENTION_SECONDS,
      metadata: { contentType: "image/png" },
    });
  }
}

export async function getReport(env: Env, id: string): Promise<StoredReport | null> {
  if (!isValidReportId(id)) return null;
  return env.REPORTS.get<StoredReport>(reportKey(id), "json");
}

export async function getScreenshotBody(env: Env, id: string): Promise<ReadableStream | null> {
  if (!isValidReportId(id)) return null;
  return env.REPORTS.get(screenshotKey(id), "stream");
}

export async function deleteReport(env: Env, id: string): Promise<boolean> {
  if (!isValidReportId(id)) return false;
  const key = reportKey(id);
  // A bounded `list()` on the exact key, not a `get()`: existence shouldn't
  // cost a read of a report that can be up to 5 MB.
  const probe = await env.REPORTS.list({ prefix: key, limit: 1 });
  if (probe.keys.length === 0 || probe.keys[0].name !== key) return false;
  await Promise.all([env.REPORTS.delete(key), env.REPORTS.delete(screenshotKey(id))]);
  return true;
}

export interface ListedReport {
  id: string;
  hasScreenshot: boolean;
}

// Oldest first for free: ids are time-prefixed and KV lists a prefix in
// ascending key order, same as R2 did.
export async function listReportIds(env: Env, limit: number): Promise<ListedReport[]> {
  const entries: ListedReport[] = [];
  let cursor: string | undefined;
  do {
    const page = await env.REPORTS.list<ReportMetadata>({
      prefix: REPORT_PREFIX,
      cursor,
      limit: Math.min(1000, limit - entries.length),
    });
    for (const key of page.keys) {
      entries.push({ id: key.name.slice(REPORT_PREFIX.length), hasScreenshot: key.metadata?.has_screenshot === true });
      if (entries.length >= limit) return entries;
    }
    cursor = page.list_complete ? undefined : page.cursor;
  } while (cursor);
  return entries;
}

async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

// Hashed rather than used verbatim: report_id is arbitrary caller text and
// must never become an unescaped KV key segment.
async function idempotencyKey(installId: string, reportId: string): Promise<string> {
  return `${IDEMPOTENCY_PREFIX}${installId}/${await sha256Hex(reportId)}`;
}

export async function lookupIdempotent(env: Env, installId: string, reportId: string): Promise<string | null> {
  const data = await env.REPORTS.get<{ id: string }>(await idempotencyKey(installId, reportId), "json");
  return data?.id ?? null;
}

export async function recordIdempotent(env: Env, installId: string, reportId: string, id: string): Promise<void> {
  const key = await idempotencyKey(installId, reportId);
  // Same TTL as a report: once the report it dedupes has expired, there is
  // nothing left to dedupe against.
  await env.REPORTS.put(key, JSON.stringify({ id, created_at: new Date().toISOString() }), {
    expirationTtl: RETENTION_SECONDS,
  });
}
