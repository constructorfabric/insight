import { describe, expect, it } from "vitest";

import i18n from "@/i18n";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";

import { metricsViewFilename, metricsViewMatrix } from "./metrics-view-export";

const RANGE = { from: "2026-07-01", to: "2026-07-31" };
const PERIOD = "2026-07-01 – 2026-07-31";

function result(
  key: string,
  label: string,
  values: Array<[string, number | null]>,
  over: Partial<NormalizedMetricResult> = {},
): NormalizedMetricResult {
  return {
    metric_key: key,
    label,
    unit: null,
    computation: "sum",
    format: "integer",
    direction: "higher_is_better",
    period: {
      view: "period",
      values: values.map(([entity_id, value]) => ({ entity_id, value })),
    },
    ...over,
  } as NormalizedMetricResult;
}

const jane = { entityId: "p-jane", displayName: "Jane Doe" };
const sam = { entityId: "p-sam", displayName: "Sam Smith" };

describe("metricsViewMatrix", () => {
  it("writes one row per person, in the order given, with each metric as a column", () => {
    const byKey = new Map([
      ["git.commits", result("git.commits", "Commits", [["p-jane", 4], ["p-sam", 9]])],
      ["collab.messages", result("collab.messages", "Messages", [["p-jane", 6], ["p-sam", 2]])],
    ]);

    const matrix = metricsViewMatrix({
      people: [sam, jane],
      keys: ["collab.messages", "git.commits"],
      byKey,
      roles: new Map(),
      range: RANGE,
    });

    expect(matrix).toEqual({
      columns: ["Person", "Period", "Messages", "Commits"],
      rows: [
        ["Sam Smith", PERIOD, 2, 9],
        ["Jane Doe", PERIOD, 6, 4],
      ],
    });
  });

  it("titles the person, role and period columns from the translations", () => {
    const original = {
      person: i18n.t("common.table.person"),
      role: i18n.t("common.table.role"),
      period: i18n.t("common.table.period"),
    };
    i18n.addResourceBundle(
      "en",
      "translation",
      { common: { table: { person: "Persona", role: "Rol", period: "Periodo" } } },
      true,
      true,
    );

    try {
      const matrix = metricsViewMatrix({
        people: [jane],
        keys: [],
        byKey: new Map(),
        roles: new Map([["p-jane", "Engineer"]]),
        range: RANGE,
      });

      expect(matrix.columns).toEqual(["Persona", "Rol", "Periodo"]);
    } finally {
      i18n.addResourceBundle("en", "translation", { common: { table: original } }, true, true);
    }
  });

  it("leaves a cell empty where the page shows no value", () => {
    const byKey = new Map([
      [
        "git.commits",
        result("git.commits", "Commits", [
          ["p-jane", null],
          ["p-sam", Number.NaN],
        ]),
      ],
    ]);

    const matrix = metricsViewMatrix({
      people: [jane, sam, { entityId: "p-none", displayName: "Nobody Yet" }],
      keys: ["git.commits", "git.missing"],
      byKey,
      roles: new Map(),
      range: RANGE,
    });

    expect(matrix.columns).toEqual(["Person", "Period", "Commits", "git.missing"]);
    expect(matrix.rows.map((row) => row.slice(2))).toEqual([
      [null, null],
      [null, null],
      [null, null],
    ]);
  });

  it.each([
    { format: "integer", unit: null, value: 4.6, want: 5, header: "Commits" },
    { format: "decimal", unit: null, value: 2.34, want: 2.3, header: "Commits" },
    { format: "percent", unit: null, value: 45.4, want: 45, header: "Commits (%)" },
    { format: "currency", unit: null, value: 1200.4, want: 1200, header: "Commits ($)" },
    { format: "decimal", unit: "h", value: 2.25, want: 2.25, header: "Commits (h)" },
    { format: "decimal", unit: "h", value: 1 + 1 / 3, want: 1.33, header: "Commits (h)" },
    { format: "integer", unit: "lines", value: 237, want: 237, header: "Commits (lines)" },
  ] as const)(
    "writes a $format value in $unit as the number the page shows, its unit in the header: $value",
    ({ format, unit, value, want, header }) => {
      const byKey = new Map([
        ["git.commits", result("git.commits", "Commits", [["p-jane", value]], { format, unit })],
      ]);

      const matrix = metricsViewMatrix({
        people: [jane],
        keys: ["git.commits"],
        byKey,
        roles: new Map(),
        range: RANGE,
      });

      expect(matrix.columns[2], `header for ${format}/${unit}`).toBe(header);
      expect(matrix.rows[0]?.[2], `value for ${format}/${unit}`).toBe(want);
    },
  );

  it.each([
    { label: "Commits", unit: "commits", header: "Commits" },
    { label: "PRs opened", unit: "PRs", header: "PRs opened" },
    { label: "Meeting hours", unit: "h", header: "Meeting hours (h)" },
    { label: "Chat messages per day", unit: "messages/day", header: "Chat messages per day (messages/day)" },
  ])("names the unit once in the header: $label in $unit", ({ label, unit, header }) => {
    const byKey = new Map([
      ["m", result("m", label, [["p-jane", 1]], { unit, format: unit === "h" ? "decimal" : "integer" })],
    ]);

    const matrix = metricsViewMatrix({
      people: [jane],
      keys: ["m"],
      byKey,
      roles: new Map(),
      range: RANGE,
    });

    expect(matrix.columns[2]).toBe(header);
  });

  it("adds each person's role when anyone in the file has one", () => {
    const matrix = metricsViewMatrix({
      people: [jane, sam],
      keys: [],
      byKey: new Map(),
      roles: new Map([["p-sam", "Software Engineer"]]),
      range: RANGE,
    });

    expect(matrix).toEqual({
      columns: ["Person", "Role", "Period"],
      rows: [
        ["Jane Doe", null, PERIOD],
        ["Sam Smith", "Software Engineer", PERIOD],
      ],
    });
  });

  it("leaves the role column out when nobody in the file has a role", () => {
    const matrix = metricsViewMatrix({
      people: [jane],
      keys: [],
      byKey: new Map(),
      roles: new Map([["p-sam", "Software Engineer"]]),
      range: RANGE,
    });

    expect(matrix.columns).toEqual(["Person", "Period"]);
  });
});

describe("metricsViewFilename", () => {
  it.each([
    ["csv", "metrics-view_2026-07-01_2026-07-31.csv"],
    ["xlsx", "metrics-view_2026-07-01_2026-07-31.xlsx"],
  ] as const)("names the %s file after the period", (extension, want) => {
    expect(metricsViewFilename(RANGE, extension)).toBe(want);
  });
});
