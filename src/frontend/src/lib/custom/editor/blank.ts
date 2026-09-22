import { read } from "./document";
import type { Field, Shape } from "./describe";

/** What a new entry of a list starts as, by the shape of an entry. */
export function blank(shape: Shape, document: unknown = {}): unknown {
  switch (shape.of) {
    case "list":
      return [];
    case "record":
      return seeded(shape.fields, document);
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
  return seeded(fields, {});
}

/**
 * A new record with the choices its description says to open on.
 *
 * INVARIANT: only a variant the description names is opened on. Picking
 * whichever happens to be written first would decide for a reader wherever a
 * kind meant to ask them.
 */
function seeded(
  fields: readonly Field[],
  document: unknown
): Record<string, unknown> {
  const held: Record<string, unknown> = {};

  for (const field of fields) {
    if (field.shape.of === "record") {
      const within = seeded(field.shape.fields, document);
      if (Object.keys(within).length > 0) held[field.name] = within;
      continue;
    }

    if (field.shape.of !== "variants") continue;

    const opens = opening(field.shape.starts, document);
    if (opens === undefined || !(opens in field.shape.variants)) continue;

    if (field.shape.recorded === undefined) {
      held[opens] = "";
    } else {
      held[field.shape.recorded] = opens;
    }
  }

  return held;
}

/** The variant to open on, read from the description or from the document. */
function opening(
  starts: Extract<Shape, { of: "variants" }>["starts"],
  document: unknown
): string | undefined {
  if (starts === undefined || typeof starts === "string") return starts;

  const said = read(document, starts.by);

  return typeof said === "string" ? starts.then[said] : undefined;
}
