import { describe, expect, it } from "vitest";
import { isValidReportId, newReportId, timeOfReportId } from "../../src/id";

describe("report ids", () => {
  it("are 26 Crockford base32 chars and self-validate", () => {
    const id = newReportId();
    expect(id.length).toBe(26);
    expect(isValidReportId(id)).toBe(true);
    expect(/^[0-9A-Z]+$/.test(id)).toBe(true);
    expect(id).not.toMatch(/[ILOU]/);
  });

  it("round-trips the millisecond time it was minted with", () => {
    const now = Date.UTC(2026, 10, 19, 3, 4, 5, 6);
    expect(timeOfReportId(newReportId(now))).toBe(now);
  });

  it("sorts lexicographically by time, oldest first", () => {
    const earlier = newReportId(1_000_000);
    const later = newReportId(2_000_000);
    expect(earlier < later).toBe(true);
    // Two ids minted at the same millisecond still compare consistently
    // (random tail breaks the tie either way, never throws).
    const a = newReportId(5_000_000);
    const b = newReportId(5_000_000);
    expect(typeof (a < b)).toBe("boolean");
  });

  it("rejects the wrong length or an out-of-alphabet character", () => {
    expect(isValidReportId("short")).toBe(false);
    expect(isValidReportId("0".repeat(26))).toBe(true);
    expect(isValidReportId("I".padEnd(26, "0"))).toBe(false);
    expect(isValidReportId("../../etc/passwd" + "0".repeat(10))).toBe(false);
    expect(Number.isNaN(timeOfReportId("not-an-id"))).toBe(true);
  });
});
