import { useState } from "react";
import { ArrowDown, ArrowUp, ChevronDown } from "lucide-react";

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
  ordering,
  onShow,
  onOrder,
}: {
  fields: readonly DeclaredField[];
  records: readonly DatasetRecord[];
  /** The declared fields to draw, in declaration order. */
  shown: readonly string[];
  ordering: Ordering;
  onShow: (names: string[]) => void;
  onOrder: (ordering: Ordering) => void;
}) {
  const drawn = fields.filter((field) => shown.includes(field.name));

  return (
    <div className="flex flex-col gap-3">
      <Columns fields={fields} shown={shown} onShow={onShow} />

      <div className="overflow-x-auto">
        <Table>
          <TableHeader>
            <TableRow>
              <Sortable
                name={ARRIVED}
                label="Received"
                ordering={ordering}
                onOrder={onOrder}
              />
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
            {records.map((record) => (
              <Row key={record.id} record={record} fields={drawn} />
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

/** Which of the declared fields the table draws. */
function Columns({
  fields,
  shown,
  onShow,
}: {
  fields: readonly DeclaredField[];
  shown: readonly string[];
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
            Columns · {shown.length} of {fields.length}
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

  return (
    <TableHead>
      <button
        type="button"
        className="inline-flex items-center gap-1 font-mono hover:underline"
        aria-label={`Order by ${label}`}
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
            <ArrowDown className="size-3" aria-label="descending" />
          ) : (
            <ArrowUp className="size-3" aria-label="ascending" />
          )
        ) : null}
      </button>
    </TableHead>
  );
}

/** One record, with the whole of it a click away. */
function Row({
  record,
  fields,
}: {
  record: DatasetRecord;
  fields: readonly DeclaredField[];
}) {
  const [open, setOpen] = useState(false);

  return (
    <>
      <TableRow
        role="button"
        tabIndex={0}
        aria-expanded={open}
        aria-label={`Show the whole record received at ${record.received_at}`}
        className="cursor-pointer"
        onClick={() => setOpen((was) => !was)}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            setOpen((was) => !was);
          }
        }}
      >
        <TableCell className="font-mono whitespace-nowrap">
          {record.received_at}
        </TableCell>
        {fields.map((field) => (
          <TableCell key={field.name} className="max-w-64 truncate font-mono">
            {cell(record.raw_data, field)}
          </TableCell>
        ))}
      </TableRow>
      {open ? (
        <TableRow>
          <TableCell colSpan={fields.length + 1}>
            <pre className={cn(TEXT_BODY, "overflow-x-auto font-mono")}>
              {JSON.stringify(record.raw_data, null, 2)}
            </pre>
          </TableCell>
        </TableRow>
      ) : null}
    </>
  );
}

/** What a declared field holds in this record, as a cell shows it. */
function cell(payload: unknown, field: DeclaredField): string {
  const value = read(payload, field.path.split("."));
  if (value === undefined || value === null) return "";
  if (typeof value === "object") return JSON.stringify(value);

  return String(value);
}
