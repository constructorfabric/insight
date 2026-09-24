import { read } from "./document";

/** A warehouse table as the service addresses it. */
export interface TableAddress {
  database: string;
  table: string;
}

/**
 * The table a document names, split as the service splits it: `database.table`
 * written in one property, or a bare table beside a `database` of its own. A
 * bare table with no database is addressed by its name alone.
 */
export function tableAddress(
  document: Record<string, unknown>,
  at: { table: string; database: string }
): TableAddress | undefined {
  const written = read(document, [at.table]);
  if (typeof written !== "string" || written === "") return undefined;

  const dot = written.indexOf(".");
  const qualified =
    dot > 0 && dot < written.length - 1 && !written.includes(".", dot + 1);
  if (qualified) {
    return { database: written.slice(0, dot), table: written.slice(dot + 1) };
  }

  const database = read(document, [at.database]);
  return {
    database: typeof database === "string" ? database : "",
    table: written,
  };
}

/** `database.table`, or the bare table where no database is known. */
export function spelled({ database, table }: TableAddress): string {
  return database === "" ? table : `${database}.${table}`;
}
