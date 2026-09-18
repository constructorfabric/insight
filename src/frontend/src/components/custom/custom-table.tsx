import type { MetricResult } from "@/api/custom-client";
import { groupedNumber, unitFor } from "@/components/custom/chart-format";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

export interface CustomTableProps {
  result: MetricResult;
}

/** Beyond this the card is a scroll bar, and the DOM pays for every row. */
const SHOWN = 200;

/**
 * One cell.
 *
 * A value that is a URL is the row's way out - to the pull request, the issue,
 * the page it counted - so it is a link rather than text to copy by hand.
 * Nothing is composed here: only a value that already is an absolute http(s)
 * URL becomes one.
 */
function Cell({ value, unit }: { value: unknown; unit: string }) {
  if (unit) return <>{groupedNumber(value, unit)}</>;

  const text = String(value);
  if (!/^https?:\/\/\S+$/.test(text)) return <>{text}</>;

  return (
    <a
      href={text}
      target="_blank"
      rel="noreferrer noopener"
      className="underline decoration-dotted underline-offset-4"
      onClick={(event) => event.stopPropagation()}
    >
      {text}
    </a>
  );
}

export function CustomTable({ result }: CustomTableProps) {
  const rows = result.rows.slice(0, SHOWN);

  return (
    <>
      {result.rows.length > SHOWN ? (
        <p className={cn(TEXT_LABEL, "mb-2")}>
          First {groupedNumber(SHOWN)} of {groupedNumber(result.rows.length)}{" "}
          rows
        </p>
      ) : null}
      <Table>
        <TableHeader>
          <TableRow>
            {result.columns.map((column) => (
              <TableHead key={column}>{column}</TableHead>
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((row, rowIndex) => (
            <TableRow key={rowIndex}>
              {result.columns.map((column, columnIndex) => (
                <TableCell key={columnIndex}>
                  {columnIndex < row.length ? (
                    <Cell
                      value={row[columnIndex]}
                      unit={unitFor(result.percents, column)}
                    />
                  ) : (
                    ""
                  )}
                </TableCell>
              ))}
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </>
  );
}
