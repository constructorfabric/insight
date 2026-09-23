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
 * the rest - and that holds however deep it sits, so a metric's fields
 * survive a change of source while the declared field each one read does not.
 */
export function switched(
  shape: VariantsShape,
  record: Record<string, unknown>,
  to: string
): Record<string, unknown> {
  const incoming = byName(shape.variants[to] ?? []);
  // Every other variant's fields, the record's own variant last so that it
  // wins: that is the shape what the record holds actually has.
  const outgoing = byName([
    ...Object.entries(shape.variants)
      .filter(([name]) => name !== to)
      .flatMap(([, fields]) => fields),
    ...(shape.variants[variantOf(shape, record)] ?? []),
  ]);

  const kept = carried(outgoing, incoming, record, shape.recorded);

  if (to === "") return kept;
  if (shape.recorded !== undefined) return { ...kept, [shape.recorded]: to };
  return to in kept ? kept : { ...kept, [to]: "" };
}

/** One value carried from the shape it was written under into another. */
function reconciled(from: Shape, to: Shape, value: unknown): unknown {
  if (from.of === "record" && to.of === "record") {
    const kept = carried(
      byName(from.fields),
      byName(to.fields),
      asRecord(value)
    );

    // A record left holding nothing says nothing, and one half-filled with
    // what the new shape cannot read is refused on sending: the property goes
    // and the rows come back empty, ready to be filled.
    return Object.keys(kept).length === 0 ? undefined : kept;
  }

  if (from.of === "list" && to.of === "list") {
    const entries = Array.isArray(value) ? value : [];

    // An entry is a row the reader added, so it stays even where nothing
    // inside it survives: a list must not silently shorten. Only a record
    // empties to nothing; a scalar entry is carried as written, `null` and
    // all, since the reader may have written it through the text view.
    return entries.map((entry) => {
      const settled = reconciled(from.entry, to.entry, entry);

      return settled === undefined ? {} : settled;
    });
  }

  return value;
}

/** The properties of one record that the incoming shape can still hold. */
function carried(
  from: ReadonlyMap<string, Field>,
  to: ReadonlyMap<string, Field>,
  record: Record<string, unknown>,
  skip?: string
): Record<string, unknown> {
  const kept: Record<string, unknown> = {};

  for (const [name, held] of Object.entries(record)) {
    if (name === skip) continue;

    const into = to.get(name);
    if (into === undefined) {
      // A property no shape here knows is the document's own and stays; one
      // the outgoing shape asked for leaves with it.
      if (!from.has(name)) kept[name] = held;
      continue;
    }

    const was = from.get(name);
    const next =
      was === undefined ? held : reconciled(was.shape, into.shape, held);
    if (next !== undefined) kept[name] = next;
  }

  return kept;
}

function byName(fields: readonly Field[]): Map<string, Field> {
  return new Map(fields.map((field) => [field.name, field]));
}
