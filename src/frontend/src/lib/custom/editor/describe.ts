import type { EditableKind } from "@/api/custom-client";

// INVARIANT: a description is the only thing that says what a kind's document
// may hold. Nothing here is inferred from a stored body or an example, so the
// editor offers the same fields for a definition that exists and one that does not.

export type Shape =
  | { of: "text"; placeholder?: string }
  | { of: "longText" }
  | { of: "number" }
  /** With `alone`, only one entry of the enclosing list may carry it. */
  | { of: "flag"; alone?: true }
  /**
   * A value offered rather than asked for: what a list's entries are called
   * under one property, or the fields the dataset named at the root declares,
   * plus whatever else is always admissible.
   */
  | {
      of: "pick";
      from: { list: string; property: string } | { dataset: string };
      also?: readonly string[];
    }
  /**
   * A value whose kind another property decides: a filter's `value` by its
   * `type`, or - when the sibling `named` is a field the dataset at the root
   * declares - by that field's declared type, which the service compares with.
   */
  | { of: "typed"; by: string; declared?: { dataset: string; named: string } }
  | { of: "choice"; options: readonly string[] }
  | { of: "reference"; to: EditableKind }
  | { of: "list"; entry: Shape; entryLabel: string }
  | { of: "record"; fields: readonly Field[] }
  /**
   * A variant's fields sit beside the choice, where the document holds them.
   * With `recorded`, that property holds the variant's key; without it, the
   * variant is the one whose own field the record carries.
   */
  | {
      of: "variants";
      recorded?: string;
      variants: Readonly<Record<string, readonly Field[]>>;
      /**
       * Which variant a new one opens on, where opening on none would leave
       * a reader answering a question before they can begin. Either a
       * variant's name, or the property at the document's root that decides
       * it. Absent asks the reader, which is the default: a choice made for
       * them is a choice they were never offered.
       */
      starts?: string | { by: Path; then: Readonly<Record<string, string>> };
    };

export interface Field {
  name: string;
  label: string;
  shape: Shape;
  required?: boolean;
  hint?: string;
}

export interface Description {
  kind: EditableKind;
  noun: string;
  fields: readonly Field[];
}

/** A path into a document, as a violation names it: `fields[0].type`. */
export type Path = readonly (string | number)[];

export function spell(path: Path): string {
  return path.reduce<string>(
    (written, segment) =>
      typeof segment === "number"
        ? `${written}[${segment}]`
        : written === ""
          ? segment
          : `${written}.${segment}`,
    ""
  );
}
