import type { Path } from "./describe";

// INVARIANT: there is one document, not two copies. The fields and the text are
// two views of it, and whichever was edited last is believed; text that does not
// parse leaves the document as it stands.
export interface Held {
  document: Record<string, unknown>;
  text: string;
  unparsed?: string;
}

export function written(document: Record<string, unknown>): string {
  return JSON.stringify(document, null, 2);
}

export function hold(document: Record<string, unknown> = {}): Held {
  return { document, text: written(document) };
}

// `undefined` removes the property rather than sending a null: a property the
// document does not carry and one carrying nothing are different documents.
export function change(held: Held, path: Path, value: unknown): Held {
  const document = put(held.document, path, value) as Record<string, unknown>;

  return { document, text: written(document) };
}

// A document must be an object: the fields have nowhere to put a list or a number.
export function retype(held: Held, text: string): Held {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text) as unknown;
  } catch (error) {
    return { ...held, text, unparsed: (error as Error).message };
  }

  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    return { ...held, text, unparsed: "A definition is an object." };
  }

  return { document: parsed as Record<string, unknown>, text };
}

export function sendable(held: Held): boolean {
  return held.unparsed === undefined;
}

export function read(document: unknown, path: Path): unknown {
  return path.reduce<unknown>((at, segment) => {
    if (at === null || typeof at !== "object") return undefined;
    if (typeof segment === "number") {
      return Array.isArray(at) ? at[segment] : undefined;
    }
    return (at as Record<string, unknown>)[segment];
  }, document);
}

// A missing step is created as the next segment needs it - an index makes a
// list, a name makes an object - so a field can be filled before what holds it exists.
function put(at: unknown, path: Path, value: unknown): unknown {
  const [segment, ...rest] = path;
  if (segment === undefined) return value;

  if (typeof segment === "number") {
    const list = Array.isArray(at) ? [...at] : [];
    const next = put(list[segment], rest, value);
    if (rest.length === 0 && value === undefined) {
      list.splice(segment, 1);
      return list;
    }
    list[segment] = next;
    return list;
  }

  const record: Record<string, unknown> =
    at !== null && typeof at === "object" && !Array.isArray(at)
      ? { ...(at as Record<string, unknown>) }
      : {};
  const next = put(record[segment], rest, value);
  if (rest.length === 0 && value === undefined) {
    delete record[segment];
    return record;
  }
  record[segment] = next;
  return record;
}
