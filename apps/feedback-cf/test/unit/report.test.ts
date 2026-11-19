import { describe, expect, it } from "vitest";
import { decodeScreenshot, MAX_TEXT_CHARS, parseReportBody, ValidationError } from "../../src/report";

const valid = () => ({
  schema: 1,
  install_id: "4e3f6f1a-9b7e-4a3a-8f1a-6f3a9b7e4a3a",
  kind: "bug",
  text: "it broke",
  client: { version: "0.18.0" },
  bundle: { host_count: 1 },
});

describe("parseReportBody", () => {
  it("accepts the minimal required shape", () => {
    const body = parseReportBody(valid());
    expect(body.schema).toBe(1);
    expect(body.kind).toBe("bug");
    expect(body.report_id).toBeUndefined();
    expect(body.email).toBeUndefined();
  });

  it("carries the optional fields through untouched", () => {
    const body = parseReportBody({ ...valid(), report_id: "r-1", email: "a@b.com", screenshot_png_b64: "AA==" });
    expect(body.report_id).toBe("r-1");
    expect(body.email).toBe("a@b.com");
    expect(body.screenshot_png_b64).toBe("AA==");
  });

  it("is permissive about unknown fields inside bundle and client", () => {
    const body = parseReportBody({ ...valid(), bundle: { anything: { nested: [1, 2, 3] } }, client: { weird: true } });
    expect(body.bundle).toEqual({ anything: { nested: [1, 2, 3] } });
    expect(body.client).toEqual({ weird: true });
  });

  const breakers: Array<[string, () => unknown]> = [
    ["not an object", () => "a string"],
    ["array body", () => []],
    ["wrong schema", () => ({ ...valid(), schema: 2 })],
    [
      "missing schema",
      () => {
        const { schema, ...rest } = valid();
        return rest;
      },
    ],
    ["bad install_id", () => ({ ...valid(), install_id: "not-a-uuid" })],
    ["bad kind", () => ({ ...valid(), kind: "feature-request" })],
    ["empty text", () => ({ ...valid(), text: "" })],
    ["text over the limit", () => ({ ...valid(), text: "x".repeat(MAX_TEXT_CHARS + 1) })],
    ["client not an object", () => ({ ...valid(), client: "nope" })],
    ["client is an array", () => ({ ...valid(), client: [] })],
    ["bundle not an object", () => ({ ...valid(), bundle: 42 })],
    ["empty report_id", () => ({ ...valid(), report_id: "" })],
    ["report_id over the limit", () => ({ ...valid(), report_id: "x".repeat(201) })],
    ["email not a string", () => ({ ...valid(), email: 12345 })],
    ["screenshot not a string", () => ({ ...valid(), screenshot_png_b64: 12345 })],
  ];

  for (const [name, breaker] of breakers) {
    it(`rejects ${name}`, () => {
      expect(() => parseReportBody(breaker())).toThrow(ValidationError);
    });
  }

  it("accepts exactly MAX_TEXT_CHARS", () => {
    expect(() => parseReportBody({ ...valid(), text: "x".repeat(MAX_TEXT_CHARS) })).not.toThrow();
  });
});

describe("decodeScreenshot", () => {
  it("decodes valid base64 to bytes", () => {
    expect(decodeScreenshot("AAEC")).toEqual(new Uint8Array([0, 1, 2]));
  });

  it("refuses text that is not base64", () => {
    expect(() => decodeScreenshot("not base64!! 🎉")).toThrow(ValidationError);
  });
});
