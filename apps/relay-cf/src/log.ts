// The whole log surface. Lifecycle events and error codes only: never a
// payload, a header, a request path, a nonce, a signature or a client address.
export type LogEvent =
  | "config_invalid"
  | "server_registered"
  | "server_superseded"
  | "server_closed"
  | "server_heartbeat_expired"
  | "server_drain_ended"
  | "registration_refused"
  | "tofu_recorded"
  | "client_opened"
  | "client_closed"
  | "client_backlogged"
  | "server_backlogged"
  | "stream_refused"
  | "hello_rate_limited"
  | "protocol_error"
  | "relay_fault";

export interface LogFields {
  server_id?: string;
  trunk_id?: string;
  client_trunk_id?: string;
  stream_id?: number;
  code?: string;
  reason?: string;
}

export function log(event: LogEvent, fields: LogFields = {}): void {
  console.log(JSON.stringify({ event, ...fields }));
}
