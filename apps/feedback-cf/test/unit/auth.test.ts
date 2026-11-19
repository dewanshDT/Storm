import { describe, expect, it } from "vitest";
import { authorized } from "../../src/auth";
import type { Env } from "../../src/env";

const envWith = (token: string | undefined): Env => ({ FEEDBACK: {} as never, RATE_LIMITER: {} as never, SYNC_TOKEN: token });

function request(headers: Record<string, string> = {}): Request {
  return new Request("https://feedback.test/v1/reports", { headers });
}

describe("authorized", () => {
  it("accepts the exact bearer token", () => {
    expect(authorized(request({ authorization: "Bearer s3cr3t" }), envWith("s3cr3t"))).toBe(true);
  });

  it("refuses a missing header, wrong scheme, wrong token or unset secret", () => {
    expect(authorized(request(), envWith("s3cr3t"))).toBe(false);
    expect(authorized(request({ authorization: "Basic s3cr3t" }), envWith("s3cr3t"))).toBe(false);
    expect(authorized(request({ authorization: "Bearer wrong" }), envWith("s3cr3t"))).toBe(false);
    expect(authorized(request({ authorization: "Bearer s3cr3t" }), envWith(undefined))).toBe(false);
  });

  it("refuses a token differing only in length", () => {
    expect(authorized(request({ authorization: "Bearer s3cr3" }), envWith("s3cr3t"))).toBe(false);
    expect(authorized(request({ authorization: "Bearer s3cr3tt" }), envWith("s3cr3t"))).toBe(false);
  });
});
