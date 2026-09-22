import type { Field, Shape } from "./describe";

/** What a new entry of a list starts as, by the shape of an entry. */
export function blank(shape: Shape): unknown {
  switch (shape.of) {
    case "list":
      return [];
    case "record":
      return seeded(shape.fields);
    case "variants":
      return {};
    case "flag":
      return false;
    case "number":
      return 0;
    case "text":
    case "longText":
    case "choice":
    case "reference":
    case "pick":
    case "typed":
      return "";
  }
}

/**
 * What a definition of this kind starts as before anything is written.
 *
 * A kind that cannot be stored without a choice starts with it made, rather
 * than refusing the first save over a question the reader was never asked.
 */
export function starting(fields: readonly Field[]): Record<string, unknown> {
  return seeded(fields);
}

/**
 * A new record with the choices it cannot do without already made.
 *
 * A choice that must be made anyway starts on its first option, so that a new
 * entry offers something to fill in rather than a question to answer first.
 * Variants are transparent, so the chosen one's properties sit in the record
 * beside everything else - which is where the document holds them.
 */
function seeded(fields: readonly Field[]): Record<string, unknown> {
  const held: Record<string, unknown> = {};

  for (const field of fields) {
    if (field.required !== true) continue;

    if (field.shape.of === "record") {
      const within = seeded(field.shape.fields);
      if (Object.keys(within).length > 0) held[field.name] = within;
      continue;
    }

    if (field.shape.of !== "variants") continue;

    const [first] = Object.keys(field.shape.variants);
    if (first === undefined) continue;

    if (field.shape.recorded === undefined) {
      held[first] = "";
    } else {
      held[field.shape.recorded] = first;
    }
  }

  return held;
}
