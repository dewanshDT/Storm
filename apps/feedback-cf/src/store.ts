import type { Env } from "./env";
import { isValidReportId, timeOfReportId } from "./id";
import { log } from "./log";
import type { StoredReport } from "./report";

const REPORTS_PREFIX = "reports/";

export function reportKey(id: string): string {
  return `${REPORTS_PREFIX}${id}/report.json`;
}

export function screenshotKey(id: string): string {
  return `${REPORTS_PREFIX}${id}/screenshot.png`;
}

export async function putReport(env: Env, report: StoredReport, screenshot: Uint8Array | undefined): Promise<void> {
  await env.FEEDBACK.put(reportKey(report.id), JSON.stringify(report), {
    httpMetadata: { contentType: "application/json" },
  });
  if (screenshot) {
    await env.FEEDBACK.put(screenshotKey(report.id), screenshot, {
      httpMetadata: { contentType: "image/png" },
    });
  }
}

export async function getReport(env: Env, id: string): Promise<StoredReport | null> {
  if (!isValidReportId(id)) return null;
  const obj = await env.FEEDBACK.get(reportKey(id));
  if (!obj) return null;
  return obj.json<StoredReport>();
}

export async function hasScreenshot(env: Env, id: string): Promise<boolean> {
  return (await env.FEEDBACK.head(screenshotKey(id))) !== null;
}

export async function getScreenshotBody(env: Env, id: string): Promise<ReadableStream | null> {
  if (!isValidReportId(id)) return null;
  const obj = await env.FEEDBACK.get(screenshotKey(id));
  return obj ? obj.body : null;
}

export async function deleteReport(env: Env, id: string): Promise<boolean> {
  if (!isValidReportId(id)) return false;
  const existing = await env.FEEDBACK.head(reportKey(id));
  if (!existing) return false;
  await env.FEEDBACK.delete([reportKey(id), screenshotKey(id)]);
  return true;
}

// Oldest first for free: ids are time-prefixed and R2 lists a prefix in
// ascending key order.
export async function listReportIds(env: Env, limit: number): Promise<string[]> {
  const ids: string[] = [];
  let cursor: string | undefined;
  do {
    const page = await env.FEEDBACK.list({ prefix: REPORTS_PREFIX, delimiter: "/", cursor });
    for (const prefix of page.delimitedPrefixes) {
      ids.push(prefix.slice(REPORTS_PREFIX.length, -1));
      if (ids.length >= limit) return ids;
    }
    cursor = page.truncated ? page.cursor : undefined;
  } while (cursor);
  return ids;
}

const IDEMPOTENCY_PREFIX = "idempotency/";

async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

// Hashed rather than used verbatim: report_id is arbitrary caller text and
// must never become an unescaped R2 key segment.
async function idempotencyKey(installId: string, reportId: string): Promise<string> {
  return `${IDEMPOTENCY_PREFIX}${installId}/${await sha256Hex(reportId)}.json`;
}

export async function lookupIdempotent(env: Env, installId: string, reportId: string): Promise<string | null> {
  const obj = await env.FEEDBACK.get(await idempotencyKey(installId, reportId));
  if (!obj) return null;
  const data = await obj.json<{ id: string }>();
  return data.id;
}

export async function recordIdempotent(env: Env, installId: string, reportId: string, id: string): Promise<void> {
  const key = await idempotencyKey(installId, reportId);
  await env.FEEDBACK.put(key, JSON.stringify({ id, created_at: new Date().toISOString() }));
}

// Unsynced reports are dropped after 30 days; the sync service deletes a
// report as soon as it lands in the vault, so anything still here past the
// cutoff was never synced.
export const RETENTION_MS = 30 * 24 * 60 * 60 * 1000;

export async function purgeExpired(env: Env, now: number = Date.now()): Promise<number> {
  const cutoff = now - RETENTION_MS;
  let cursor: string | undefined;
  let deleted = 0;
  do {
    const page = await env.FEEDBACK.list({ prefix: REPORTS_PREFIX, delimiter: "/", cursor });
    for (const prefix of page.delimitedPrefixes) {
      const id = prefix.slice(REPORTS_PREFIX.length, -1);
      const createdAt = timeOfReportId(id);
      if (Number.isFinite(createdAt) && createdAt < cutoff) {
        await env.FEEDBACK.delete([reportKey(id), screenshotKey(id)]);
        deleted++;
      }
    }
    cursor = page.truncated ? page.cursor : undefined;
  } while (cursor);
  deleted += await purgeExpiredIdempotency(env, cutoff);
  if (deleted > 0) log("retention_purged", { count: deleted });
  return deleted;
}

// Without this an idempotency marker outlives every report it dedupes and
// `idempotency/` grows forever. R2's own `uploaded` timestamp is used since
// a marker has no id to decode a time from, and reading every body just to
// find `created_at` would cost a GET per marker instead of one LIST.
async function purgeExpiredIdempotency(env: Env, cutoff: number): Promise<number> {
  let cursor: string | undefined;
  let deleted = 0;
  do {
    const page = await env.FEEDBACK.list({ prefix: IDEMPOTENCY_PREFIX, cursor });
    const stale = page.objects.filter((o) => o.uploaded.getTime() < cutoff).map((o) => o.key);
    if (stale.length > 0) {
      await env.FEEDBACK.delete(stale);
      deleted += stale.length;
    }
    cursor = page.truncated ? page.cursor : undefined;
  } while (cursor);
  return deleted;
}
