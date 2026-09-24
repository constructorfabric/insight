import { read } from "./document";

/** A warehouse table as the service addresses it. */
export interface TableAddress {
  database: string;
  table: string;
}

/** What the service will take as a name: letters, digits and underscore. */
const IDENTIFIER = /^[A-Za-z0-9_]{1,128}$/;

/** Whether a written name carries its database, as `database.table`. */
export function qualifies(written: unknown): boolean {
  if (typeof written !== "string") return false;
  const dot = written.indexOf(".");

  return (
    dot > 0 &&
    IDENTIFIER.test(written.slice(0, dot)) &&
    IDENTIFIER.test(written.slice(dot + 1))
  );
}

/**
 * The table a written name and a database mean together.
 *
 * INVARIANT: the service reads a `database` of its own in preference to the
 * one a qualified name carries, and never splits the name beside one. The
 * editor splits by the same rule, or it would offer the columns of a table
 * the metric does not read.
 */
export function addressOf(
  written: unknown,
  database: unknown
): TableAddress | undefined {
  if (typeof written !== "string" || written === "") return undefined;
  if (typeof database === "string" && database !== "") {
    return { database, table: written };
  }
  if (!qualifies(written)) return { database: "", table: written };

  const dot = written.indexOf(".");

  return { database: written.slice(0, dot), table: written.slice(dot + 1) };
}

/** The same, out of the document the editor holds. */
export function tableAddress(
  document: Record<string, unknown>,
  at: { table: string; database: string }
): TableAddress | undefined {
  return addressOf(read(document, [at.table]), read(document, [at.database]));
}

/**
 * The body as the service is given it: the table named once, as
 * `database.table`.
 *
 * The service also takes a `database` of its own beside a bare name, and a
 * metric stored before the editor offered the catalogue is written that way.
 * The editor asks for one name, so a body settles on the one form the moment
 * anything about it is saved. The two compile to the same query.
 */
export function dotted(
  document: Record<string, unknown>
): Record<string, unknown> {
  const database = document.database;
  const table = document.table;
  if (typeof database !== "string" || database === "") return document;
  if (typeof table !== "string" || table === "") return document;

  const settled: Record<string, unknown> = {
    ...document,
    table: `${database}.${table}`,
  };
  delete settled.database;

  return settled;
}

/** `database.table`, or the bare table where no database is known. */
export function spelled({ database, table }: TableAddress): string {
  return database === "" ? table : `${database}.${table}`;
}
