import { CustomApiError } from "@/api/custom-client";

import type { Field, Shape } from "./describe";

/**
 * Where the service's refusal belongs on the form.
 *
 * INVARIANT: a violation naming something the form does not offer stays in
 * `loose`. The service checks more than the editor asks for, so one dropped
 * for naming an unoffered path would leave a form that refuses and says
 * nothing.
 */
export interface Placed {
  /** What was said about each path, indices and all. */
  at: ReadonlyMap<string, string>;
  /** Everything else the service said, in the order it said it. */
  loose: string[];
}

interface Violation {
  field?: string;
  description?: string;
}

interface Refusal {
  detail?: string;
  context?: { violations?: Violation[] };
}

/** A path with its list indices dropped: `fields[2].type` is `fields[].type`. */
function anyEntry(path: string): string {
  return path.replaceAll(/\[\d+\]/g, "[]");
}

/**
 * Every path a description offers, with list indices dropped.
 *
 * Built from the description alone: which rows a document happens to hold
 * decides where a violation sits, never whether the form has a place for it.
 */
export function offers(fields: readonly Field[], base = ""): Set<string> {
  const paths = new Set<string>();

  for (const field of fields) {
    if (field.shape.of === "variants") {
      for (const path of within(field.shape, base)) paths.add(path);
      continue;
    }

    const here = base === "" ? field.name : `${base}.${field.name}`;
    paths.add(here);
    for (const path of within(field.shape, here)) paths.add(path);
  }

  return paths;
}

function within(shape: Shape, here: string): Set<string> {
  if (shape.of === "record") return offers(shape.fields, here);
  if (shape.of === "variants") {
    const recorded = shape.recorded;
    const choice =
      recorded === undefined
        ? []
        : [here === "" ? recorded : `${here}.${recorded}`];

    return new Set([
      ...choice,
      ...offers(Object.values(shape.variants).flat(), here),
    ]);
  }
  if (shape.of === "list") {
    const entry = `${here}[]`;
    return new Set([entry, ...within(shape.entry, entry)]);
  }

  return new Set();
}

/** The service's refusal, split between the fields it names and the rest. */
export function place(error: unknown, fields: readonly Field[]): Placed {
  const at = new Map<string, string>();
  const loose: string[] = [];

  if (!(error instanceof CustomApiError)) {
    return { at, loose };
  }

  const body = error.body as Refusal | null;
  const offered = offers(fields);

  for (const violation of body?.context?.violations ?? []) {
    const said = violation.description ?? "";
    const where = violation.field ?? "";

    if (said === "") continue;

    if (offered.has(anyEntry(where)) && !at.has(where)) at.set(where, said);
    else loose.push(where === "" ? said : `${where}: ${said}`);
  }

  if (at.size === 0 && loose.length === 0 && body?.detail) {
    loose.push(body.detail);
  }

  return { at, loose };
}
