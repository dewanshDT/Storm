import { describe, expect, it } from "vitest";
import * as indexModule from "../../src/index";

// Regression for the Worker that would not start: workerd validates every
// top-level export of the entry module besides `default`, and rejects
// anything that is not a function, a class, or an `ExportedHandler` — a
// plain `export const MAX_BODY_BYTES = …` once lived here and failed with
// "Incorrect type for map entry 'GLOBAL_DAILY_LIMIT': the provided value is
// not of type 'function or ExportedHandler'" under `wrangler dev` (and would
// have under a real deploy), while `npm test`'s vitest-pool-workers never
// exercises that validation and stayed green throughout. Constants the tests
// need live in `src/limits.ts` instead; nothing here guards against a future
// `export const` landing back in `index.ts` except this shape check.
describe("index.ts's top-level exports (workerd's module validation)", () => {
  it("exports only `default` and function/class values", () => {
    for (const [name, value] of Object.entries(indexModule)) {
      if (name === "default") continue;
      expect(typeof value === "function", `export '${name}' is a ${typeof value}, not a function or class`).toBe(
        true,
      );
    }
  });
});
