import type { Field, Shape } from "./describe";

export type VariantsShape = Extract<Shape, { of: "variants" }>;

export function asRecord(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

/** Which variant a record is, by what it records or by what it carries. */
export function variantOf(
  shape: VariantsShape,
  record: Record<string, unknown>
): string {
  if (shape.recorded !== undefined) {
    const said = record[shape.recorded];
    return typeof said === "string" ? said : "";
  }

  return (
    Object.keys(shape.variants).find((name) => record[name] !== undefined) ?? ""
  );
}

/**
 * The record with another variant chosen.
 *
 * What the incoming variant also asks for stays: a chart's metric is the same
 * metric whatever it is drawn as. What only the outgoing variant asked for
 * goes, since the service reads the record as its new variant and refuses
 * the rest.
 */
export function switched(
  shape: VariantsShape,
  record: Record<string, unknown>,
  to: string
): Record<string, unknown> {
  const incoming = new Set(names(shape.variants[to] ?? []));
  const outgoing = new Set(
    Object.values(shape.variants)
      .flat()
      .map((field) => field.name)
      .filter((name) => !incoming.has(name))
  );
  const kept = Object.fromEntries(
    Object.entries(record).filter(
      ([name]) => !outgoing.has(name) && name !== shape.recorded
    )
  );

  if (to === "") return kept;
  if (shape.recorded !== undefined) return { ...kept, [shape.recorded]: to };

  return to in kept ? kept : { ...kept, [to]: "" };
}

function names(fields: readonly Field[]): string[] {
  return fields.map((field) => field.name);
}
