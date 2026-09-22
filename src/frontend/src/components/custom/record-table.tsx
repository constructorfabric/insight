import { useId, useState } from "react";
import { ArrowDown, ArrowUp, ChevronDown, ChevronRight } from "lucide-react";

import type { DatasetRecord, DeclaredField } from "@/api/custom-client";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { pathSegments } from "@/lib/custom/dataset-path";
import { read } from "@/lib/custom/editor/document";
import { TEXT_BODY, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/** The column every record carries, whatever the dataset declares. */
export const ARRIVED = "received_at";

export interface Ordering {
  by: string;
  descending: boolean;
}

/**
 * A dataset's records as a table of its declared fields.
 *
 * INVARIANT: the columns come from the declaration, not from the keys a
 * record happens to carry. A record missing a declared value shows an empty
 * cell, and a key nothing declares is in the record a row opens, not in a
 * column of its own.
 */
export function RecordTable({
  fields,
  records,
  shown,
  arrived,
  ordering,
  onShow,
  onOrder,
}: {
  fields: readonly DeclaredField[];
  records: readonly DatasetRecord[];
  /** The declared fields to draw, in declaration order. */
  shown: readonly string[];
  /**
   * Whether these rows were sent into the dataset, and so carry the instant
   * they arrived. A row of a relation the warehouse builds was not sent and
   * nothing stamped it.
   *
   * INVARIANT: this comes from the declaration. Reading it off the rows would
   * take the column away from a stream dataset whenever a page came back
   * empty — which a reader can reach by paging past the end.
   */
  arrived: boolean;
  ordering: Ordering;
  onShow: (names: string[]) => void;
  onOrder: (ordering: Ordering) => void;
}) {
  const drawn = fields.filter((field) => shown.includes(field.name));

  return (
    <div className="flex flex-col gap-3">
      <Columns fields={fields} shown={shown} drawn={drawn} onShow={onShow} />

      <Table>
        <TableHeader>
          <TableRow>
            <TableHead className="w-0" />
            {arrived ? (
              <Sortable
                name={ARRIVED}
                label="Received"
                ordering={ordering}
                onOrder={onOrder}
              />
            ) : null}
            {drawn.map((field) => (
              <Sortable
                key={field.name}
                name={field.name}
                label={field.name}
                ordering={ordering}
                onOrder={onOrder}
              />
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {records.map((record, at) => (
            <Row
              key={record.id ?? at}
              record={record}
              at={at}
              fields={drawn}
              arrived={arrived}
            />
          ))}
        </TableBody>
      </Table>
    </div>
  );
}

/** Which of the declared fields the table draws. */
function Columns({
  fields,
  shown,
  drawn,
  onShow,
}: {
  fields: readonly DeclaredField[];
  shown: readonly string[];
  /** What is actually drawn: a held name the declaration dropped is not. */
  drawn: readonly DeclaredField[];
  onShow: (names: string[]) => void;
}) {
  const toggle = (name: string) =>
    onShow(
      shown.includes(name)
        ? shown.filter((held) => held !== name)
        : fields
            .map((field) => field.name)
            .filter((held) => held === name || shown.includes(held))
    );

  return (
    <Collapsible>
      <CollapsibleTrigger
        render={
          <Button variant="ghost" size="sm" className="self-start">
            <ChevronDown className="size-4" />
            Columns · {drawn.length} of {fields.length}
          </Button>
        }
      />
      <CollapsibleContent>
        <div className="mt-2 grid gap-x-6 gap-y-2 rounded-md border border-border p-3 sm:grid-cols-2 lg:grid-cols-3">
          {fields.map((field) => (
            <label
              key={field.name}
              className={cn(TEXT_LABEL, "flex items-center gap-2")}
            >
              <Checkbox
                checked={shown.includes(field.name)}
                onCheckedChange={() => toggle(field.name)}
              />
              <span className="font-mono">{field.name}</span>
              <span className="text-muted-foreground">{field.type}</span>
            </label>
          ))}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}

function Sortable({
  name,
  label,
  ordering,
  onOrder,
}: {
  name: string;
  label: string;
  ordering: Ordering;
  onOrder: (ordering: Ordering) => void;
}) {
  const chosen = ordering.by === name;
  const sorted = chosen ? (ordering.descending ? "descending" : "ascending") : "none";

  return (
    <TableHead aria-sort={sorted}>
      <button
        type="button"
        className="inline-flex items-center gap-1 font-mono hover:underline"
        aria-label={
          chosen ? `Order by ${label}, now ${sorted}` : `Order by ${label}`
        }
        onClick={() =>
          onOrder({
            by: name,
            descending: chosen ? !ordering.descending : true,
          })
        }
      >
        {label}
        {chosen ? (
          ordering.descending ? (
            <ArrowDown className="size-3" aria-hidden />
          ) : (
            <ArrowUp className="size-3" aria-hidden />
          )
        ) : null}
      </button>
    </TableHead>
  );
}

/** One record, with the whole of it a click away. */
function Row({
  record,
  at,
  fields,
  arrived,
}: {
  record: DatasetRecord;
  /** Where the row sits on the page, for one that carries no stamp. */
  at: number;
  fields: readonly DeclaredField[];
  /** Whether this table draws the instant a row arrived. */
  arrived: boolean;
}) {
  const [open, setOpen] = useState(false);
  const whole = useId();

  return (
    <>
      <TableRow
        className="cursor-pointer"
        onClick={() => setOpen((was) => !was)}
      >
        <TableCell className="w-0 pr-0">
          <button
            type="button"
            id={`${whole}-trigger`}
            aria-expanded={open}
            aria-controls={open ? whole : undefined}
            aria-label={
              record.received_at === undefined
                ? `${open ? "Hide" : "Show"} the whole of row ${at + 1}`
                : `${open ? "Hide" : "Show"} the whole of the row received at ${record.received_at}`
            }
            className="flex items-center text-muted-foreground"
            onClick={(event) => {
              event.stopPropagation();
              setOpen((was) => !was);
            }}
          >
            {open ? (
              <ChevronDown className="size-4" aria-hidden />
            ) : (
              <ChevronRight className="size-4" aria-hidden />
            )}
          </button>
        </TableCell>
        {arrived ? (
          <TableCell className="font-mono whitespace-nowrap">
            {record.received_at}
          </TableCell>
        ) : null}
        {fields.map((field) => {
          const held = cell(record.raw_data, field);
          return (
            <TableCell
              key={field.name}
              title={held || undefined}
              className="max-w-64 truncate font-mono"
            >
              {held}
            </TableCell>
          );
        })}
      </TableRow>
      {open ? (
        <TableRow>
          <TableCell colSpan={fields.length + (arrived ? 2 : 1)}>
            <pre
              id={whole}
              aria-labelledby={`${whole}-trigger`}
              className={cn(TEXT_BODY, "overflow-x-auto font-mono")}
            >
              {JSON.stringify(record.raw_data, null, 2)}
            </pre>
          </TableCell>
        </TableRow>
      ) : null}
    </>
  );
}

/**
 * What a declared field holds in this row, as a cell shows it.
 *
 * INVARIANT: the path is split the way the service splits it, and an absent
 * value stands in the way the service stands it in. A table that reads a
 * declaration differently from the metrics over it is worse than no table.
 *
 * A field of a dataset over a relation names a column, and the service hands
 * back a row already keyed by the field names it declares — so the value sits
 * under the field's own name, not under a path.
 */
function cell(payload: unknown, field: DeclaredField): string {
  const at = "path" in field ? pathSegments(field.path) : [field.name];
  const value = read(payload, at);
  if (value === undefined || value === null || value === "") {
    return field.absent_value ?? "";
  }
  if (typeof value === "object") return JSON.stringify(value);

  return String(value);
}
