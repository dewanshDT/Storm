import { authorized } from "./auth";
import type { Env } from "./env";
import { jsonResponse, methodNotAllowed, notFound, unauthorized } from "./http";
import { newReportId } from "./id";
import { RateLimiter } from "./limiter";
import { log } from "./log";
import { decodeScreenshot, parseReportBody, ValidationError, type StoredReport } from "./report";
import {
  deleteReport,
  getReport,
  getScreenshotBody,
  hasScreenshot,
  listReportIds,
  lookupIdempotent,
  purgeExpired,
  putReport,
  recordIdempotent,
} from "./store";

export { RateLimiter };

const MAX_BODY_BYTES = 5 * 1024 * 1024;
const RATE_LIMIT = 20;
const RATE_WINDOW_MS = 60 * 60 * 1000;
const DEFAULT_LIST_LIMIT = 50;
const MAX_LIST_LIMIT = 500;

const REPORTS_PATH = "/v1/reports";
const SCREENSHOT_RE = /^\/v1\/reports\/([^/]+)\/screenshot$/;
const ID_RE = /^\/v1\/reports\/([^/]+)$/;

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);

    if (url.pathname === REPORTS_PATH) {
      if (request.method === "POST") return createReport(request, env);
      if (request.method === "GET") return listReports(request, env, url);
      return methodNotAllowed();
    }

    const screenshotMatch = SCREENSHOT_RE.exec(url.pathname);
    if (screenshotMatch) {
      if (request.method !== "GET") return methodNotAllowed();
      return getScreenshot(request, env, decodeURIComponent(screenshotMatch[1]));
    }

    const idMatch = ID_RE.exec(url.pathname);
    if (idMatch) {
      if (request.method !== "DELETE") return methodNotAllowed();
      return removeReport(request, env, decodeURIComponent(idMatch[1]));
    }

    return notFound();
  },

  async scheduled(event: ScheduledController, env: Env, _ctx: ExecutionContext): Promise<void> {
    await purgeExpired(env, event.scheduledTime);
  },
};

async function createReport(request: Request, env: Env): Promise<Response> {
  const contentLength = request.headers.get("content-length");
  if (contentLength !== null && Number(contentLength) > MAX_BODY_BYTES) {
    log("report_rejected", { status: 413, reason: "content-length" });
    return jsonResponse({ error: "payload too large" }, 413);
  }

  const bytes = new Uint8Array(await request.arrayBuffer());
  if (bytes.byteLength > MAX_BODY_BYTES) {
    log("report_rejected", { status: 413, reason: "body too large", bytes: bytes.byteLength });
    return jsonResponse({ error: "payload too large" }, 413);
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(new TextDecoder().decode(bytes));
  } catch {
    log("report_rejected", { status: 400, reason: "not json" });
    return jsonResponse({ error: "body must be JSON" }, 400);
  }

  let body;
  try {
    body = parseReportBody(parsed);
  } catch (e) {
    const reason = e instanceof ValidationError ? e.message : "invalid body";
    log("report_rejected", { status: 400, reason });
    return jsonResponse({ error: reason }, 400);
  }

  // A bad screenshot is a 400 like any other validation failure — caught
  // before idempotency or the rate limiter so a client's encoding bug never
  // costs it budget or gets deduplicated away.
  let screenshot: Uint8Array | undefined;
  if (body.screenshot_png_b64 !== undefined) {
    try {
      screenshot = decodeScreenshot(body.screenshot_png_b64);
    } catch (e) {
      const reason = e instanceof ValidationError ? e.message : "invalid screenshot";
      log("report_rejected", { status: 400, reason });
      return jsonResponse({ error: reason }, 400);
    }
  }

  // Idempotent resend: the client's own report_id, already stored for this
  // install_id, is answered with the id it got the first time — before the
  // rate limiter ever sees it, so a retry never spends the caller's budget.
  if (body.report_id) {
    const existing = await lookupIdempotent(env, body.install_id, body.report_id);
    if (existing) {
      log("report_idempotent_replay", { id: existing, bytes: bytes.byteLength });
      return jsonResponse({ id: existing }, 200);
    }
  }

  const limiter = env.RATE_LIMITER.get(env.RATE_LIMITER.idFromName(body.install_id));
  const taken = await limiter.take(RATE_LIMIT, RATE_WINDOW_MS);
  if (!taken.allowed) {
    log("report_rate_limited", { status: 429 });
    return jsonResponse({ error: "rate limited" }, 429, {
      "Retry-After": String(Math.max(1, Math.ceil(taken.retryAfterMs / 1000))),
    });
  }

  const id = newReportId();
  const stored: StoredReport = {
    id,
    schema: 1,
    install_id: body.install_id,
    report_id: body.report_id,
    kind: body.kind,
    text: body.text,
    email: body.email,
    client: body.client,
    bundle: body.bundle,
    received_at: new Date().toISOString(),
  };
  await putReport(env, stored, screenshot);
  if (body.report_id) await recordIdempotent(env, body.install_id, body.report_id, id);

  log("report_stored", { id, bytes: bytes.byteLength, has_screenshot: screenshot !== undefined });
  return jsonResponse({ id }, 201);
}

async function listReports(request: Request, env: Env, url: URL): Promise<Response> {
  if (!authorized(request, env)) {
    log("auth_failed", { status: 401, reason: "GET /v1/reports" });
    return unauthorized();
  }

  let limit = DEFAULT_LIST_LIMIT;
  const limitParam = url.searchParams.get("limit");
  if (limitParam !== null) {
    const parsed = Number(limitParam);
    if (!Number.isInteger(parsed) || parsed <= 0) {
      return jsonResponse({ error: "limit must be a positive integer" }, 400);
    }
    limit = Math.min(parsed, MAX_LIST_LIMIT);
  }

  const ids = await listReportIds(env, limit);
  const reports = [];
  for (const id of ids) {
    const report = await getReport(env, id);
    if (!report) continue;
    reports.push({ ...report, has_screenshot: await hasScreenshot(env, id) });
  }
  return jsonResponse({ reports }, 200);
}

async function getScreenshot(request: Request, env: Env, id: string): Promise<Response> {
  if (!authorized(request, env)) {
    log("auth_failed", { status: 401, reason: "GET screenshot" });
    return unauthorized();
  }
  const body = await getScreenshotBody(env, id);
  if (!body) return notFound();
  return new Response(body, { status: 200, headers: { "content-type": "image/png" } });
}

async function removeReport(request: Request, env: Env, id: string): Promise<Response> {
  if (!authorized(request, env)) {
    log("auth_failed", { status: 401, reason: "DELETE /v1/reports/:id" });
    return unauthorized();
  }
  const existed = await deleteReport(env, id);
  if (!existed) return notFound();
  log("report_deleted", { id });
  return new Response(null, { status: 204 });
}
