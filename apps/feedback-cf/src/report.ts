// Decision 162d's POST /v1/reports body, and what report.json holds in R2
// (plus `id` and `received_at`, stamped by the Worker). `client` and
// `bundle` are the caller's own shape — permissive on purpose, since 162c's
// diagnostics bundle grows independently of this Worker.
export type ReportKind = "bug" | "idea" | "other";

export interface ReportBody {
  schema: 1;
  install_id: string;
  report_id?: string;
  kind: ReportKind;
  text: string;
  email?: string;
  client: Record<string, unknown>;
  bundle: Record<string, unknown>;
  screenshot_png_b64?: string;
}

export interface StoredReport {
  id: string;
  schema: 1;
  install_id: string;
  report_id?: string;
  kind: ReportKind;
  text: string;
  email?: string;
  client: Record<string, unknown>;
  bundle: Record<string, unknown>;
  received_at: string;
}

export class ValidationError extends Error {}

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const KINDS: ReadonlySet<string> = new Set(["bug", "idea", "other"]);

export const MAX_TEXT_CHARS = 20_000;
const MAX_REPORT_ID_CHARS = 200;
const MAX_EMAIL_CHARS = 320;

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parseReportBody(value: unknown): ReportBody {
  if (!isPlainObject(value)) throw new ValidationError("body must be a JSON object");
  if (value.schema !== 1) throw new ValidationError("schema must be 1");
  if (typeof value.install_id !== "string" || !UUID_RE.test(value.install_id)) {
    throw new ValidationError("install_id must be a UUID");
  }
  if (typeof value.kind !== "string" || !KINDS.has(value.kind)) {
    throw new ValidationError("kind must be bug, idea or other");
  }
  if (typeof value.text !== "string" || value.text.length === 0 || value.text.length > MAX_TEXT_CHARS) {
    throw new ValidationError(`text must be 1..${MAX_TEXT_CHARS} characters`);
  }
  if (!isPlainObject(value.client)) throw new ValidationError("client must be an object");
  if (!isPlainObject(value.bundle)) throw new ValidationError("bundle must be an object");

  let reportId: string | undefined;
  if (value.report_id !== undefined) {
    if (typeof value.report_id !== "string" || value.report_id.length === 0 || value.report_id.length > MAX_REPORT_ID_CHARS) {
      throw new ValidationError("report_id must be a non-empty string");
    }
    reportId = value.report_id;
  }

  let email: string | undefined;
  if (value.email !== undefined) {
    if (typeof value.email !== "string" || value.email.length > MAX_EMAIL_CHARS) {
      throw new ValidationError("email must be a string");
    }
    email = value.email;
  }

  let screenshot: string | undefined;
  if (value.screenshot_png_b64 !== undefined) {
    if (typeof value.screenshot_png_b64 !== "string") throw new ValidationError("screenshot_png_b64 must be a string");
    screenshot = value.screenshot_png_b64;
  }

  return {
    schema: 1,
    install_id: value.install_id,
    report_id: reportId,
    kind: value.kind as ReportKind,
    text: value.text,
    email,
    client: value.client,
    bundle: value.bundle,
    screenshot_png_b64: screenshot,
  };
}

export function decodeScreenshot(base64: string): Uint8Array {
  let binary: string;
  try {
    binary = atob(base64);
  } catch {
    throw new ValidationError("screenshot_png_b64 is not valid base64");
  }
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}
