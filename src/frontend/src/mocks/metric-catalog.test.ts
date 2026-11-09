import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

const REGISTRY = join(
  __dirname,
  "../../../backend/services/analytics/src/domain/metric_definitions/registry.yaml",
);

const FIELDS = [
  "entity_type",
  "label",
  "short_label",
  "description",
  "explanation",
  "unit",
  "format",
  "direction",
  "computation",
] as const;

type Field = (typeof FIELDS)[number];
type CatalogEntry = { metric_key: string } & Partial<Record<Field, string>>;

function isField(name: string): name is Field {
  return (FIELDS as readonly string[]).includes(name);
}

function blockScalar(style: string, lines: string[]): string {
  const text = lines.map((line) => line.trim());
  return (style.startsWith("|") ? text.join("\n") : text.join(" ")).trim();
}

function inlineScalar(raw: string): string {
  if (raw.startsWith("'") && raw.endsWith("'")) return raw.slice(1, -1).replaceAll("''", "'");
  if (raw.startsWith('"')) return JSON.parse(raw) as string;
  if (raw.startsWith("!")) return raw.slice(1).split(/\s/)[0]!;
  return raw;
}

function catalogFrom(yaml: string): CatalogEntry[] {
  const lines = yaml.split("\n");
  const entries: CatalogEntry[] = [];
  let index = lines.indexOf("metrics:") + 1;

  while (index < lines.length) {
    const line = lines[index]!;
    index += 1;

    const start = /^ {2}- metric_key: (.+)$/.exec(line);
    if (start) {
      entries.push({ metric_key: start[1]!.trim() });
      continue;
    }

    const field = /^ {4}(\w+):\s?(.*)$/.exec(line);
    const entry = entries.at(-1);
    if (!field || !entry || !isField(field[1]!)) continue;

    const raw = field[2]!.trim();
    if (/^[>|][-+]?$/.test(raw)) {
      const block: string[] = [];
      while (index < lines.length && /^( {6}|\s*$)/.test(lines[index]!)) {
        block.push(lines[index]!);
        index += 1;
      }
      entry[field[1]!] = blockScalar(raw, block);
    } else {
      entry[field[1]!] = inlineScalar(raw);
    }
  }

  return entries;
}

describe("mock metric catalog", () => {
  it("mirrors the analytics registry, so mock metrics carry their real meaning", async () => {
    const catalog = catalogFrom(readFileSync(REGISTRY, "utf8"));

    expect(catalog.length, "should find every registry metric").toBeGreaterThan(100);
    await expect(`${JSON.stringify(catalog, null, 2)}\n`).toMatchFileSnapshot(
      "./metric-catalog.json",
    );
  });
});
