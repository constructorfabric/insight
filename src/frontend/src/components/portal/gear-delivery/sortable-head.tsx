import { ArrowDown, ArrowUp, ChevronsUpDown } from "lucide-react";

import { TableHead } from "@/components/ui/table";
import { ariaSort, nextSort, type SortState } from "@/lib/gears/sort";

/** A column header that sorts its table; the icon names the current order. */
export function SortableHead<Key extends string>({
  column,
  label,
  sort,
  onSort,
  numeric,
  className,
}: {
  column: Key;
  label: string;
  sort: SortState<Key>;
  onSort: (next: SortState<Key>) => void;
  numeric?: boolean;
  className?: string;
}) {
  const active = sort.key === column;
  const Icon = !active ? ChevronsUpDown : sort.direction === "asc" ? ArrowUp : ArrowDown;

  return (
    <TableHead aria-sort={ariaSort(sort, column)} className={className}>
      <button
        type="button"
        onClick={() => onSort(nextSort(sort, column))}
        className={`flex w-full items-center gap-1 ${
          numeric ? "justify-end" : ""
        } ${active ? "text-foreground" : ""}`}
      >
        {label}
        <Icon className="size-3 shrink-0 opacity-60" />
      </button>
    </TableHead>
  );
}
