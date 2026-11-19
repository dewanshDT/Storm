// The whole log surface. Ids, byte counts and status only: never a report's
// text, email, client or bundle contents (decision 162d).
export type LogEvent =
  | "report_stored"
  | "report_idempotent_replay"
  | "report_rejected"
  | "report_rate_limited"
  | "report_capacity_exceeded"
  | "report_deleted"
  | "auth_failed"
  | "retention_purged";

export interface LogFields {
  id?: string;
  bytes?: number;
  has_screenshot?: boolean;
  status?: number;
  reason?: string;
  count?: number;
}

export function log(event: LogEvent, fields: LogFields = {}): void {
  console.log(JSON.stringify({ event, ...fields }));
}
