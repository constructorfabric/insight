import { describe, expect, it } from "vitest";

import { csvCell, matrixToCsv } from "@/lib/export/matrix";

describe("csvCell", () => {
  it("defuses a value a spreadsheet would run as a formula", () => {
    // A name or a metric label starting with one of these is data, not a
    // command, and the file is opened by a person who did not write it.
    expect(csvCell("=1+1")).toBe("'=1+1");
    expect(csvCell("+7")).toBe("'+7");
    expect(csvCell("-lead")).toBe("'-lead");
    expect(csvCell("@handle")).toBe("'@handle");
  });

  it("quotes what would otherwise break the row apart", () => {
    expect(csvCell('say "hi"')).toBe('"say ""hi"""');
    expect(csvCell("a,b")).toBe('"a,b"');
    expect(csvCell("two\nlines")).toBe('"two\nlines"');
  });

  it("writes an absent value as empty and keeps a measured zero", () => {
    expect(csvCell(null)).toBe("");
    expect(csvCell(undefined)).toBe("");
    expect(csvCell(0)).toBe("0");
  });

  it("drops a non-finite number rather than writing Infinity into a cell", () => {
    expect(csvCell(Number.POSITIVE_INFINITY)).toBe("");
    expect(csvCell(Number.NaN)).toBe("");
  });
});

describe("matrixToCsv", () => {
  it("writes the header first and separates rows with CRLF", () => {
    expect(
      matrixToCsv({
        columns: ["Person", "Period", "Commits"],
        rows: [
          ["Jane Doe", "2026-Q1", 4],
          ["Sam Smith", "2026-Q1", null],
        ],
      }),
    ).toBe("Person,Period,Commits\r\nJane Doe,2026-Q1,4\r\nSam Smith,2026-Q1,");
  });
});
