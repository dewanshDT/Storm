// SRP v1 control messages and the binary body-frame header (docs/srp-v1.md §3, §6).

export const PROTOCOL_VERSION = 1;
export const HEARTBEAT_INTERVAL_SECS = 15;

export type ErrorCode =
  | "protocol_error"
  | "auth_failed"
  | "server_unreachable"
  | "server_timeout"
  | "trunk_lost"
  | "rate_limited"
  | "stream_closed"
  | "trunk_superseded"
  | "stream_aborted"
  | "client_backlogged";

// One fixed message per code, never per cause (§6).
const MESSAGES: Record<ErrorCode, string> = {
  protocol_error: "protocol error",
  auth_failed: "authentication failed",
  server_unreachable: "server unreachable",
  server_timeout: "server timeout",
  trunk_lost: "trunk lost",
  rate_limited: "rate limited",
  stream_closed: "stream closed",
  trunk_superseded: "trunk superseded",
  stream_aborted: "stream aborted",
  client_backlogged: "client backlogged",
};

export class ProtocolError extends Error {
  constructor() {
    super("protocol_error");
  }
}

export type Body = Record<string, unknown>;

export interface Frame {
  type: string;
  body: Body;
}

// `v` is accepted before anything else in the frame is read (§3).
export function parseFrame(text: string): Frame {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    throw new ProtocolError();
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new ProtocolError();
  const { v, type, ...body } = value as Record<string, unknown>;
  if (v !== PROTOCOL_VERSION) throw new ProtocolError();
  if (typeof type !== "string") throw new ProtocolError();
  return { type, body };
}

export function stringField(body: Body, name: string): string {
  const value = body[name];
  if (typeof value !== "string") throw new ProtocolError();
  return value;
}

export function isStreamId(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= 0xffffffff;
}

export function streamIdOf(body: Body): number {
  const value = body.stream_id;
  if (!isStreamId(value)) throw new ProtocolError();
  return value;
}

export function control(type: string, body: Body = {}): string {
  return JSON.stringify({ v: PROTOCOL_VERSION, type, ...body });
}

export function error(code: ErrorCode): string {
  return control("ERROR", { code, message: MESSAGES[code] });
}

export function errorOnStream(code: ErrorCode, streamId: number): string {
  return control("ERROR", { code, message: MESSAGES[code], stream_id: streamId });
}

export function errorOnAttempt(code: ErrorCode, attemptId: unknown): string {
  return control("ERROR", { code, message: MESSAGES[code], attempt_id: attemptId });
}

export const PING = control("PING");
export const PONG = control("PONG");

export const closeStream = (streamId: number) => control("CLOSE", { stream_id: streamId });

export const BODY_HEADER_LEN = 5;
export const BODY_REQUEST = 0x01;
export const BODY_RESPONSE = 0x02;

// The header only; the payload is forwarded as it arrived and never read (§1).
export function parseBodyHeader(bytes: ArrayBuffer): { kind: number; streamId: number } {
  if (bytes.byteLength < BODY_HEADER_LEN) throw new ProtocolError();
  const view = new DataView(bytes);
  const kind = view.getUint8(0);
  if (kind !== BODY_REQUEST && kind !== BODY_RESPONSE) throw new ProtocolError();
  return { kind, streamId: view.getUint32(1, false) };
}
