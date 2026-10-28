import type { Witness } from "@/api/alerts-types";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { conditionColumns } from "@/lib/alerts/condition";
import { cellText, matchedText } from "@/lib/alerts/describe";
import { TEXT_BODY, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export function WitnessTable({ witness }: { witness: Witness }) {
  return (
    <div className="flex flex-col gap-1">
      <p className={TEXT_BODY}>{matchedText(witness)}</p>
      {witness.rows.length > 0 ? (
        <Table>
          <TableHeader>
            <TableRow>
              {witness.columns.map((column) => (
                <TableHead key={column} className="font-mono">
                  {column}
                </TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {witness.rows.map((row, index) => (
              <TableRow key={index}>
                {witness.columns.map((column, at) => (
                  <TableCell
                    key={column}
                    className={cn(
                      typeof row[at] === "number" && "tabular-nums"
                    )}
                  >
                    {cellText(row[at])}
                  </TableCell>
                ))}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      ) : null}
    </div>
  );
}

// INVARIANT: worded as the notification words its rows, "core: open 12 · infra: open 15".
function rowLine(
  witness: Witness,
  row: readonly unknown[],
  tested: ReadonlySet<string>
): string {
  const groups: string[] = [];
  const read: string[] = [];
  witness.columns.forEach((column, at) => {
    if (tested.has(column)) read.push(`${column} ${cellText(row[at])}`);
    else groups.push(cellText(row[at]));
  });

  const values = read.join(", ");
  return groups.length > 0 ? `${groups.join(" ")}: ${values}` : values;
}

export function WitnessLines({
  witness,
  condition,
  contextColumns,
}: {
  witness: Witness;
  condition: unknown;
  /** Read as the condition's columns are, name then value. */
  contextColumns: readonly string[];
}) {
  const tested = new Set([...conditionColumns(condition), ...contextColumns]);

  return (
    <span className="flex flex-col gap-0.5">
      <span className={TEXT_BODY}>{matchedText(witness)}</span>
      {witness.rows.length > 0 ? (
        <span className={cn(TEXT_LABEL, "break-words")}>
          {witness.rows.map((row) => rowLine(witness, row, tested)).join(" · ")}
        </span>
      ) : null}
    </span>
  );
}
