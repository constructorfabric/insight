import { CustomApiError } from "@/api/custom-client";

import type { Field, Path, Shape } from "./describe";
import { spell } from "./describe";
import { read } from "./document";
import { asRecord, variantOf } from "./variant";

/**
 * Where the service's refusal belongs on the form.
 *
 * INVARIANT: `at` holds only paths the form draws a row for, given this very
 * document — a chosen variant's fields, the entries a list has. Anything else
 * the service said is in `loose`, so a form never refuses in silence.
 */
export interface Placed {
  at: ReadonlyMap<string, string>;
  loose: string[];
}

interface FieldViolation {
  field?: string;
  description?: string;
}

interface PreconditionViolation {
  subject?: string;
  description?: string;
}

/**
 * The two refusal bodies the service sends: a 400 names places in the
 * document, a 409 names things outside it.
 */
interface Refusal {
  detail?: string;
  context?: {
    field_violations?: FieldViolation[];
    violations?: PreconditionViolation[];
  };
}

/** Every path the form draws a row for, given this document. */
export function offers(
  fields: readonly Field[],
  document: unknown,
  at: Path = []
): Set<string> {
  const paths = new Set<string>();

  for (const field of fields) {
    if (field.shape.of === "variants") {
      for (const path of chosen(field.shape, document, at)) paths.add(path);
      continue;
    }

    const here = [...at, field.name];
    paths.add(spell(here));
    for (const path of within(field.shape, document, here)) paths.add(path);
  }

  return paths;
}

function within(shape: Shape, document: unknown, at: Path): Set<string> {
  if (shape.of === "record") return offers(shape.fields, document, at);
  if (shape.of === "variants") return chosen(shape, document, at);
  if (shape.of === "list") return entries(shape.entry, document, at);

  return new Set();
}

function chosen(
  shape: Extract<Shape, { of: "variants" }>,
  document: unknown,
  at: Path
): Set<string> {
  const record = asRecord(read(document, at));
  const variant = shape.variants[variantOf(shape, record)] ?? [];
  const choice =
    shape.recorded === undefined ? [] : [spell([...at, shape.recorded])];

  return new Set([...choice, ...offers(variant, document, at)]);
}

function entries(entry: Shape, document: unknown, at: Path): Set<string> {
  const held = read(document, at);
  if (!Array.isArray(held)) return new Set();

  const paths = new Set<string>();
  held.forEach((_, index) => {
    const here = [...at, index];
    paths.add(spell(here));
    for (const path of within(entry, document, here)) paths.add(path);
  });

  return paths;
}

/**
 * The service's refusal, split between the rows it names and the rest.
 *
 * Nothing readable in the error — no service body at all — still says
 * something, since a form that stays silent after a failed save reads as a
 * form that saved.
 */
export function place(
  error: unknown,
  fields: readonly Field[],
  document: unknown,
  fallback: string,
  /**
   * Places the form draws itself, outside any description — the name of the
   * definition is one, and it is the refusal a first author meets most.
   */
  also: readonly string[] = []
): Placed {
  const at = new Map<string, string>();
  const loose: string[] = [];

  if (error === undefined || error === null) return { at, loose };
  if (!(error instanceof CustomApiError)) return { at, loose: [fallback] };

  const body = error.body as Refusal | null;
  const offered = offers(fields, document);
  for (const path of also) offered.add(path);

  for (const violation of body?.context?.field_violations ?? []) {
    const said = violation.description ?? "";
    const where = violation.field ?? "";
    if (said === "") continue;

    if (offered.has(where) && !at.has(where)) at.set(where, said);
    else loose.push(where === "" ? said : `${where}: ${said}`);
  }

  for (const violation of body?.context?.violations ?? []) {
    const said = violation.description ?? "";
    const about = violation.subject ?? "";
    if (said === "") continue;

    loose.push(about === "" ? said : `${about}: ${said}`);
  }

  if (at.size === 0 && loose.length === 0) loose.push(body?.detail ?? fallback);

  return { at, loose };
}
