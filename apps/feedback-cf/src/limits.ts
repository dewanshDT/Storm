// Pulled out of index.ts (decision 162d). workerd's module loader validates
// every top-level export of a Worker's entry module: besides `default`, each
// one must be a function, a class, or an `ExportedHandler` — a plain number
// or string fails with "Incorrect type for map entry '<name>': the provided
// value is not of type 'function or ExportedHandler'" and the Worker never
// starts, under `wrangler dev` or deployed. `index.test.ts` holds this.

export const MAX_BODY_BYTES = 5 * 1024 * 1024;
export const RATE_LIMIT = 20;
export const RATE_WINDOW_MS = 60 * 60 * 1000;
// Backstop for an abuser rotating install_id (caller-chosen, free to rotate).
export const IP_RATE_LIMIT = 60;
export const IP_RATE_WINDOW_MS = 60 * 60 * 1000;
export const IP_LIMITER_PREFIX = "ip:";
// A cheap cap on total stored reports a day, regardless of caller.
export const GLOBAL_DAILY_LIMIT = 2000;
export const GLOBAL_WINDOW_MS = 24 * 60 * 60 * 1000;
export const GLOBAL_LIMITER_NAME = "global";
