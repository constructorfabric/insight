import type { EditableKind } from "@/api/custom-client";

/**
 * What a kind admits, said once, so the editor can offer it.
 *
 * INVARIANT: nothing here is inferred from a stored body or an example. A
 * description is the only thing that says what a kind's document may hold, so
 * an editor built from it offers the same fields for a definition that exists
 * and one that does not.
 */

/** What one value may be. */
export type Shape =
  | { of: "text"; placeholder?: string }
  | { of: "longText" }
  | { of: "number" }
  | { of: "flag" }
  /** One of a fixed set, which the editor offers rather than asks for. */
  | { of: "choice"; options: readonly string[] }
  /** The name of a stored definition, offered from the catalogue. */
  | { of: "reference"; to: EditableKind }
  | { of: "list"; entry: Shape; entryLabel: string }
  | { of: "record"; fields: readonly Field[] }
  /**
   * A record whose fields depend on which variant it is: a widget by its type,
   * a dashboard item by what it carries.
   *
   * A variant's fields sit beside the choice, not under it, because that is
   * where the document holds them. With `recorded`, that property holds the
   * variant's key; without it, the variant is the one whose own field the
   * record carries.
   */
  | {
      of: "variants";
      recorded?: string;
      variants: Readonly<Record<string, readonly Field[]>>;
    };

/** One property of a document, as the editor offers it. */
export interface Field {
  /** The property's name in the document, which is also its path segment. */
  name: string;
  label: string;
  shape: Shape;
  required?: boolean;
  /** One line saying what it is for, where the label cannot carry it. */
  hint?: string;
}

/** Everything the editor needs to offer one kind. */
export interface Description {
  kind: EditableKind;
  /** What one of these is called, in the singular. */
  noun: string;
  fields: readonly Field[];
}

/** A path into a document, as a violation names it: `fields[0].type`. */
export type Path = readonly (string | number)[];

/** The path as the service spells it, which is how a violation is matched. */
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
