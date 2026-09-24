import { useQueries, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";

import type { EditableKind, WarehouseTable } from "@/api/custom-client";
import { FieldsView } from "@/components/custom/editor/fields-view";
import { TextView } from "@/components/custom/editor/text-view";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import type { Path } from "@/lib/custom/editor/describe";
import { spell } from "@/lib/custom/editor/describe";
import {
  change,
  hold,
  retype,
  sendable,
  type Held,
} from "@/lib/custom/editor/document";
import { DESCRIPTIONS } from "@/lib/custom/editor/kinds";
import type { TableAddress } from "@/lib/custom/editor/source";
import {
  dotted,
  qualifies,
  spelled,
  tableAddress,
} from "@/lib/custom/editor/source";
import { place } from "@/lib/custom/editor/violations";
import {
  catalogueNamesQuery,
  datasetQuery,
  tableQuery,
  tablesQuery,
  useStoreDefinition,
} from "@/queries/custom";
import { TEXT_BODY, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

const KINDS: EditableKind[] = ["datasets", "metrics", "widgets", "dashboards"];

/** Where a metric writes the table it reads, and the database beside it. */
const TABLE_AT = { table: "table", database: "database" } as const;

/**
 * The catalogue's own entry for a written name.
 *
 * A table is asked for only once the catalogue says it is there, so typing a
 * name spends no request per keystroke on tables that do not exist.
 *
 * INVARIANT: only a qualified name resolves. The service reads a bare one
 * against the warehouse's own database, which the catalogue cannot stand in
 * for - answering with the one database that happens to hold that name would
 * offer the columns of a table the metric does not read.
 */
function found(
  listed: readonly WarehouseTable[],
  named: TableAddress
): WarehouseTable | undefined {
  if (named.database === "") return undefined;

  return listed.find(
    (each) => each.database === named.database && each.table === named.table
  );
}

/**
 * What the catalogue has to say about the name written, where the reader
 * would otherwise be left guessing.
 *
 * None of it refuses the name: a table made in the last few minutes is
 * readable before it is listed, and a bare name is a shape the service takes.
 */
function notice(
  failed: boolean,
  listedOk: boolean,
  named: TableAddress | undefined,
  listed: readonly WarehouseTable[],
  address: WarehouseTable | undefined
): string | undefined {
  if (failed) {
    return "The catalogue could not be read, so nothing is offered here. A name written out in full is still stored and still runs.";
  }
  if (named === undefined || !listedOk) return undefined;

  const holders = listed.filter((each) => each.table === named.table);
  if (named.database === "" && holders.length > 0) {
    const where = holders.map((each) => each.database);

    return `A bare name reads the warehouse's own database. ${
      where.length === 1
        ? `${where[0]} holds a table called \`${named.table}\``
        : `${where.length} databases hold one: ${where.join(", ")}`
    }. Write the one you mean, as database.table.`;
  }
  if (address !== undefined) return undefined;

  return "The catalogue does not list this table. It can still be saved - a table made in the last few minutes is readable before it is listed - but a metric over a table that is not there fails when it runs.";
}

/**
 * One change to the document.
 *
 * INVARIANT: the service reads a `database` of its own in preference to the
 * one a qualified name carries, so the two standing together name a table
 * whose name holds a dot - which no warehouse has. Writing a qualified name
 * takes the database with it.
 */
function written(
  held: Held,
  path: Path,
  value: unknown,
  overTables: boolean
): Held {
  const next = change(held, path, value);
  if (!overTables || spell(path) !== TABLE_AT.table || !qualifies(value)) {
    return next;
  }

  return change(next, [TABLE_AT.database], undefined);
}

export function DefinitionEditor({
  kind,
  name,
  document,
  onStored,
}: {
  kind: EditableKind;
  /** The definition being edited. A new one has no name until it is given. */
  name?: string;
  document?: Record<string, unknown>;
  onStored: (name: string) => void;
}) {
  const description = DESCRIPTIONS[kind];
  const [called, setCalled] = useState(name ?? "");
  // A copy: the description's own object is shared by every editor this
  // session opens, and the document is edited in place from here on.
  const [held, setHeld] = useState<Held>(() =>
    hold(document ?? (description.starting && { ...description.starting }))
  );
  const [view, setView] = useState<"fields" | "text">("fields");
  const store = useStoreDefinition();

  const catalogues = useQueries({
    queries: KINDS.map((each) => catalogueNamesQuery(each)),
  });
  const stored = new Map(
    KINDS.map((each, index) => [each, catalogues[index]?.data ?? []])
  );
  const names = (of: EditableKind) => stored.get(of) ?? [];

  const reads = held.document.dataset;
  const dataset = useQuery({
    ...datasetQuery(typeof reads === "string" ? reads : ""),
    enabled: typeof reads === "string" && reads !== "",
  });
  const declared = () => dataset.data?.declaration.fields ?? [];

  // A metric over a table is offered the catalogue, and the columns of the
  // table it names.
  const overTables = kind === "metrics";
  const catalogue = useQuery({ ...tablesQuery(), enabled: overTables });
  const listed = catalogue.data?.tables ?? [];
  const named = tableAddress(held.document, TABLE_AT);
  const address = named && found(listed, named);
  const table = useQuery({
    ...tableQuery(address?.database ?? "", address?.table ?? ""),
    enabled: address !== undefined,
  });

  // What the catalogue says about the table the metric names, where that is
  // something the reader would want to know before saving and finding out.
  const notes = new Map<string, string>();
  const noticed = overTables
    ? notice(catalogue.isError, catalogue.isSuccess, named, listed, address)
    : undefined;
  if (noticed !== undefined) notes.set(TABLE_AT.table, noticed);

  const tables = () => listed.map(spelled);
  const columns = (asked: TableAddress) => {
    const resolved = found(listed, asked);
    const held = table.data;
    if (
      resolved === undefined ||
      held === undefined ||
      held.database !== resolved.database ||
      held.table !== resolved.table
    ) {
      return [];
    }

    return held.columns.map((column) => column.name);
  };

  const placed = place(
    store.error,
    description.fields,
    held.document,
    "Couldn't store it."
  );
  const given = called.trim();
  // SAFETY: the service replaces on write. A new definition given a name the
  // catalogue already holds would write over that one, so the form refuses
  // what it can see is taken; the catalogue may run a page behind, so this is
  // a guard against a slip, not the service's own check.
  const taken = name === undefined && names(kind).includes(given);

  const submit = () => {
    if (given === "" || taken || !sendable(held)) return;
    store.mutate(
      { kind, name: given, body: overTables ? dotted(held.document) : held.document },
      { onSuccess: () => onStored(given) }
    );
  };

  return (
    <form
      className="flex flex-col gap-5"
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
      // SAFETY: storing claims a name and, for a dataset, builds what holds its
      // records. Enter in a field is not that decision; only Save is.
      onKeyDown={(event) => {
        const inField = event.target instanceof HTMLInputElement;
        if (event.key === "Enter" && inField) event.preventDefault();
      }}
    >
      {name === undefined ? (
        <div className="flex flex-col gap-1">
          <label
            htmlFor="definition-name"
            className={cn(TEXT_LABEL, "font-medium")}
          >
            Name
          </label>
          <Input
            id="definition-name"
            value={called}
            aria-invalid={taken || undefined}
            aria-describedby="definition-name-hint"
            className="h-9 w-72 font-mono"
            onChange={(event) => setCalled(event.target.value)}
          />
          <p
            id="definition-name-hint"
            className={cn(TEXT_LABEL, "text-muted-foreground")}
          >
            The identifier: what the API path, other definitions and anything
            sending records call this {description.noun}. Letters, digits,{" "}
            <code>_</code> and <code>-</code>.
          </p>
          {taken ? (
            <p role="alert" className={cn(TEXT_LABEL, "text-destructive")}>
              A {description.noun} called <code>{given}</code> already exists.{" "}
              <Link
                to="/portal/custom/edit/$kind/$name"
                params={{ kind, name: given }}
                className="underline decoration-dotted underline-offset-4"
              >
                Open it
              </Link>{" "}
              instead of writing over it.
            </p>
          ) : null}
        </div>
      ) : (
        <div className="flex flex-col gap-1">
          <span className={cn(TEXT_LABEL, "font-medium")}>Name</span>
          <span className={cn(TEXT_BODY, "font-mono")}>{name}</span>
          <p className={cn(TEXT_LABEL, "text-muted-foreground")}>
            {kind === "datasets"
              ? "A dataset keeps its name: records are sent to it by this name."
              : "Renamed under Rename, below, so everything that names it follows."}
          </p>
        </div>
      )}

      {placed.loose.map((said) => (
        <p
          key={said}
          role="alert"
          className={cn(TEXT_BODY, "text-destructive")}
        >
          {said}
        </p>
      ))}

      <div className="flex items-center gap-2">
        {(["fields", "text"] as const).map((each) => (
          <Button
            key={each}
            type="button"
            variant={view === each ? "secondary" : "ghost"}
            size="sm"
            aria-pressed={view === each}
            onClick={() => setView(each)}
          >
            {each === "fields" ? "Editor" : "JSON"}
          </Button>
        ))}
      </div>

      {view === "fields" ? (
        <FieldsView
          fields={description.fields}
          at={[]}
          editing={{
            document: held.document,
            said: placed.at,
            notes,
            names,
            declared,
            tables,
            columns,
            onChange: (path: Path, value: unknown) =>
              setHeld((was) => written(was, path, value, overTables)),
          }}
        />
      ) : (
        <div className="flex flex-col gap-1">
          <TextView
            id="definition-text"
            label={`${description.noun} as text`}
            text={held.text}
            unparsed={held.unparsed}
            onChange={(text) => setHeld((was) => retype(was, text))}
          />
          {placed.at.size === 0 ? null : (
            <ul
              className={cn(
                TEXT_LABEL,
                "flex flex-col gap-0.5 text-destructive"
              )}
            >
              {[...placed.at].map(([path, said]) => (
                <li key={path} role="alert">
                  <span className="font-mono">{path}</span>: {said}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      <div className="flex items-center gap-2">
        <Button
          type="submit"
          disabled={given === "" || taken || !sendable(held) || store.isPending}
        >
          {store.isPending ? <Spinner className="size-3" /> : null}
          Save
        </Button>
        {held.unparsed ? (
          <span className={cn(TEXT_LABEL, "text-muted-foreground")}>
            The text has to parse before this can be sent.
          </span>
        ) : null}
      </div>
    </form>
  );
}
