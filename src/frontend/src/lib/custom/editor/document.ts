import type { Path } from "./describe";

/**
 * The document an editor holds, and the text beside it.
 *
 * INVARIANT: there is one document, not two copies. The fields and the text
 * are two views of it, and whichever was edited last is believed. Text that
 * does not parse leaves the document as it stands, so the fields never show
 * something nobody wrote.
 */
export interface Held {
  /** What would be sent, exactly. */
  document: Record<string, unknown>;
  /** What the text view shows, which is the document unless it was typed. */
  text: string;
  /** Why the text cannot be believed, when it cannot. */
  unparsed?: string;
}

/** The document as text, the way the editor writes it. */
export function written(document: Record<string, unknown>): string {
  return JSON.stringify(document, null, 2);
}

/** A document opened for editing: either a stored one, or nothing yet. */
export function hold(document: Record<string, unknown> = {}): Held {
  return { document, text: written(document) };
}

/**
 * The document with one place changed.
 *
 * The text follows, because the fields were edited last. A value of
 * `undefined` removes the property rather than sending a null: a property the
 * document does not carry and one carrying nothing are different documents.
 */
export function change(held: Held, path: Path, value: unknown): Held {
  const document = put(held.document, path, value) as Record<string, unknown>;

  return { document, text: written(document) };
}

/**
 * The text, as typed.
 *
 * Parsing is what decides whether the document follows. A document must be an
 * object: the service is sent one, and the fields have nowhere to put a list
 * or a number.
 */
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

/** Whether what is held can be sent at all. */
export function sendable(held: Held): boolean {
  return held.unparsed === undefined;
}

/** What sits at a path, or nothing. */
export function read(document: unknown, path: Path): unknown {
  return path.reduce<unknown>((at, segment) => {
    if (at === null || typeof at !== "object") return undefined;
    if (typeof segment === "number") {
      return Array.isArray(at) ? at[segment] : undefined;
    }
    return (at as Record<string, unknown>)[segment];
  }, document);
}

/**
 * The same document with one path set, sharing everything it did not touch.
 *
 * A missing step is created as the next segment needs it — an index makes a
 * list, a name makes an object — so a field can be filled before the thing
 * holding it exists.
 */
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
