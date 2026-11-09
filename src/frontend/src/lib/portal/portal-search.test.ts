import {
  defaultParseSearch,
  defaultStringifySearch,
  retainSearchParams,
} from "@tanstack/react-router";
import { describe, expect, it } from "vitest";

import { toISODate } from "@/api/period-to-date-range";

import {
  GLOBAL_SEARCH_KEYS,
  applySearchPatch,
  validateDashboardListSearch,
  validateDashboardSearch,
  validateExploreSearch,
  validateGlobalSearch,
  validateIdentitiesSearch,
  validateMetricsViewSearch,
} from "./portal-search";

/**
 * Search params are user-editable text and arrive from links other people
 * wrote. The contract is to DEGRADE — drop what makes no sense and fall back to
 * a computed default — never to error, because the alternative is an error
 * boundary in place of a dashboard over one mistyped character.
 */
/**
 * What a route reads: the router spreads the validated search over the parent
 * match's, and the root route's is the raw query string. A key the validator
 * drops therefore has to come back as `undefined`, not be omitted.
 */
function asMatched(raw: Record<string, unknown>): Record<string, unknown> {
  return { ...raw, ...validateGlobalSearch(raw) };
}

function isoDaysFromToday(days: number): string {
  const d = new Date();
  d.setDate(d.getDate() + days);
  return toISODate(d);
}

describe("the identities console keys", () => {
  it("normalises the picked sources and drops an empty list", () => {
    expect(validateIdentitiesSearch({ src: " gitlab,github,github" }).src).toBe(
      "github,gitlab",
    );
    expect(validateIdentitiesSearch({ src: ",," }).src).toBeUndefined();
    expect(validateIdentitiesSearch({ src: 5 }).src).toBeUndefined();
  });

  it("degrades an oversized src to at most 20 entries of at most 30 characters", () => {
    const many = Array.from({ length: 25 }, (_, i) => `s${String(i).padStart(2, "0")}`);
    const long = "x".repeat(31);
    const fits = "y".repeat(30);

    const kept = validateIdentitiesSearch({ src: [...many, long, fits].join(",") }).src?.split(",");

    expect(kept).toHaveLength(20);
    expect(kept?.every((entry) => entry.length <= 30)).toBe(true);
    expect(kept).not.toContain(long);
    expect(validateIdentitiesSearch({ src: `${long},${fits}` }).src).toBe(fits);
  });

  it("lowercases the person and stays on the console", () => {
    expect(validateIdentitiesSearch({ person: "AB12" }).person).toBe("ab12");
    expect(GLOBAL_SEARCH_KEYS).not.toContain("person");
  });
});

describe("the repository under inspection", () => {
  it("keeps a repository by its dimension value and drops an empty one", () => {
    // By VALUE, so a shared link reproduces the one that was opened even when
    // two share a display name.
    expect(validateExploreSearch({ repo: "src-a:acme/api" }).repo).toBe("src-a:acme/api");
    expect(validateExploreSearch({ repo: "" }).repo).toBeUndefined();
  });

  it("stays on the lens it was opened on", () => {
    expect(GLOBAL_SEARCH_KEYS).not.toContain("repo");
  });
});

describe("the settings every page shares", () => {
  it("lowercases the scope — an email is case-insensitive but our keys are not", () => {
    expect(validateGlobalSearch({ scope: "Head@X.COM" }).scope).toBe("head@x.com");
  });

  it("drops empty and non-string values rather than passing them on", () => {
    const out = validateGlobalSearch({ scope: "", slice: 42, period: null });
    expect(out.scope).toBeUndefined();
    expect(out.slice).toBeUndefined();
    expect(out.period).toBeUndefined();
  });

  it("are carried across every navigation", () => {
    expect([...GLOBAL_SEARCH_KEYS].sort()).toEqual(
      ["direct", "from", "period", "scope", "slice", "to"].sort(),
    );
  });

  it("omits `direct` unless it is truthy — no defaults in a shared URL", () => {
    expect(asMatched({ direct: "false" }).direct).toBeUndefined();
    expect(validateGlobalSearch({}).direct).toBeUndefined();
    expect(validateGlobalSearch({ direct: "true" }).direct).toBe(true);
  });

  it("ignores an unknown period preset", () => {
    expect(validateGlobalSearch({ period: "fortnight" }).period).toBeUndefined();
    expect(validateGlobalSearch({ period: "quarter" }).period).toBe("quarter");
  });

  describe("custom range", () => {
    it("keeps a well-formed, correctly ordered pair", () => {
      const out = validateGlobalSearch({ from: "2026-01-01", to: "2026-01-31" });
      expect(out).toMatchObject({ from: "2026-01-01", to: "2026-01-31" });
    });

    it.each([
      ["inverted", { from: "2026-07-30", to: "2026-01-01" }],
      ["longer than the maximum span", { from: "2000-01-01", to: "2026-01-01" }],
      ["past today", { from: "2026-01-01", to: isoDaysFromToday(2) }],
      ["missing its end", { from: "2026-01-01" }],
      ["missing its start", { to: "2026-01-31" }],
      ["not ISO", { from: "01/01/2026", to: "31/01/2026" }],
      ["not a calendar date", { from: "2026-13-45", to: "2026-13-46" }],
    ])("drops a range that is %s, so the page opens on the preset", (_case, raw) => {
      const search = asMatched(raw);
      expect(search.from, JSON.stringify(raw)).toBeUndefined();
      expect(search.to, JSON.stringify(raw)).toBeUndefined();
    });
  });
});

describe("Metrics view links", () => {
  it.each([
    ["keeps well-formed metric keys and drops the rest", { metrics: "git.commits,<b>,ai.cost" }, { metrics: "git.commits,ai.cost" }],
    ["keeps an empty pick", { metrics: "none" }, { metrics: "none" }],
    ["drops a list with nothing usable", { metrics: ",," }, { metrics: undefined }],
    ["keeps compared ids, lower-cased", { compare: "AB12-CD,x y" }, { compare: "ab12-cd" }],
  ])("%s", (_case, raw, expected) => {
    expect(validateMetricsViewSearch(raw)).toMatchObject(expected);
  });
});

describe("applySearchPatch", () => {
  /** The real middleware the portal routes install, over the patched result. */
  function afterRetain(
    prev: Record<string, unknown>,
    patched: Record<string, unknown>
  ): Record<string, unknown> {
    const middleware = retainSearchParams(GLOBAL_SEARCH_KEYS);
    return middleware({
      search: prev as never,
      next: (() => ({ search: patched, meta: {} })) as never,
    }) as unknown as Record<string, unknown>;
  }

  it("survives retention when a key is cleared", () => {
    // Choosing a preset clears the custom range. Deleting the keys handed them
    // back: the middleware restores whatever is absent, so the range stayed in
    // force and the preset did nothing.
    const prev = { period: "month", from: "2026-05-01", to: "2026-05-08" };
    const patched = applySearchPatch(prev, {
      period: "quarter",
      from: undefined,
      to: undefined,
    });

    expect("from" in patched).toBe(true);
    const kept = afterRetain(prev, patched);
    expect(kept.period).toBe("quarter");
    expect(kept.from).toBeUndefined();
    expect(kept.to).toBeUndefined();
    expect(validateGlobalSearch(kept)).toEqual({ period: "quarter" });
  });

  it("clears a key given an empty string or false, and leaves omitted keys alone", () => {
    const patched = applySearchPatch(
      { scope: "ab12", slice: "division", direct: true },
      { slice: "", direct: false }
    );
    expect(patched.scope).toBe("ab12");
    expect(patched.slice).toBeUndefined();
    expect(patched.direct).toBeUndefined();
  });
});

describe("the picked time range", () => {
  it("keeps a preset and an interval the server would accept", () => {
    expect(validateDashboardSearch({ range: "P30D" }).range).toBe("P30D");
    expect(validateDashboardSearch({ range: "2026-08-01/2026-09-01" }).range).toBe(
      "2026-08-01/2026-09-01",
    );
  });

  it("drops what the server would refuse, leaving the board's own default", () => {
    for (const range of [
      "P14D",
      "2026-09-02/2026-09-01",
      "2026-02-30/2026-03-01",
      "2026-09-01/2026-09-02/2026-09-03",
      "",
    ]) {
      expect(validateDashboardSearch({ range }).range, range).toBeUndefined();
    }
  });

  it("stays on the dashboard it was picked on", () => {
    expect(GLOBAL_SEARCH_KEYS).not.toContain("range");
  });
});

describe("the dashboard folder param", () => {
  it("keeps the folder a link names", () => {
    expect(validateDashboardListSearch({ folder: "unfiled" }).folder).toBe("unfiled");
    expect(validateDashboardListSearch({ folder: "" }).folder).toBeUndefined();
  });

  it("rides only on the link that names it, never into another zone", () => {
    expect(GLOBAL_SEARCH_KEYS).not.toContain("folder");
  });
});

describe("the dashboard tag param", () => {
  it("keeps every tag a link names, one or several", () => {
    expect(validateDashboardListSearch({ tag: "Ops" }).tag).toEqual(["Ops"]);
    expect(validateDashboardListSearch({ tag: ["Ops", "Platform"] }).tag).toEqual([
      "Ops",
      "Platform",
    ]);
  });

  it.each([
    ["an empty value", ""],
    ["an empty list", []],
    ["a list of empty values", ["", ""]],
    ["an object", { name: "Ops" }],
  ])("drops %s", (_case, tag) => {
    expect(validateDashboardListSearch({ tag }).tag).toBeUndefined();
  });

  it("reads the tags a hand-written link repeats, and the list the router writes", () => {
    const repeated = defaultParseSearch("?tag=Ops&tag=2026&tag=");
    const written = defaultParseSearch(
      defaultStringifySearch({ tag: ["Ops", "2026"] }),
    );

    expect(validateDashboardListSearch(repeated).tag).toEqual(["Ops", "2026"]);
    expect(validateDashboardListSearch(written).tag).toEqual(["Ops", "2026"]);
  });

  it("rides only on the link that names it, never into another zone", () => {
    expect(GLOBAL_SEARCH_KEYS).not.toContain("tag");
  });
});

describe("the dashboard ownership param", () => {
  it.each(["me", "shared"])("keeps %s", (owned) => {
    expect(validateDashboardListSearch({ owned }).owned).toBe(owned);
  });

  it.each([
    ["an empty value", ""],
    ["an unknown filter", "everyone"],
    ["a list", ["me"]],
  ])("drops %s", (_case, owned) => {
    expect(validateDashboardListSearch({ owned }).owned).toBeUndefined();
  });

  it("rides only on the link that names it, never into another zone", () => {
    expect(GLOBAL_SEARCH_KEYS).not.toContain("owned");
  });
});

describe("the expanded widget", () => {
  it("keeps a widget id and drops an empty one", () => {
    expect(validateDashboardSearch({ widget: "commits_table" }).widget).toBe(
      "commits_table",
    );
    expect(validateDashboardSearch({ widget: "" }).widget).toBeUndefined();
  });

  it("stays on the dashboard it was opened on", () => {
    expect(GLOBAL_SEARCH_KEYS).not.toContain("widget");
  });
});
