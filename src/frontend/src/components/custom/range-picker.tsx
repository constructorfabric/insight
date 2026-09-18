import { CalendarIcon } from "lucide-react";
import { useState } from "react";
import type { DateRange as DayPickerRange } from "react-day-picker";

import { toISODate } from "@/api/period-to-date-range";
import { Button } from "@/components/ui/button";
import { Calendar } from "@/components/ui/calendar";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import {
  RANGE_PRESETS,
  rangeLabel,
  toInterval,
  toRangeToken,
} from "@/lib/custom/time-range";

export interface RangePickerProps {
  /** The tokens this board offers, in the order it offers them. */
  offered: string[];
  /** The token being read now — a preset, or an interval of its own. */
  selected: string;
  onSelect: (token: string) => void;
}

/**
 * Which window a custom dashboard is read over.
 *
 * The option list belongs to the board, so this draws what it was handed and
 * nothing else. Only a token ever leaves here: the server resolves every
 * window, including a custom one, against the clock.
 */
export function RangePicker({ offered, selected, onSelect }: RangePickerProps) {
  const [open, setOpen] = useState(false);
  const custom = toInterval(selected);

  // Keyed on the selection: a `useState` initialiser runs once, so a picker
  // reopened after the window moved would apply the interval it first saw.
  const [picked, setPicked] = useState<DayPickerRange | undefined>(undefined);
  const showing =
    picked ??
    (custom
      ? {
          from: new Date(`${custom.from}T00:00:00`),
          to: new Date(`${custom.to}T00:00:00`),
        }
      : undefined);

  const presets = RANGE_PRESETS.filter((preset) =>
    offered.includes(preset.token),
  );

  return (
    <ToggleGroup
      value={[custom ? "custom" : selected]}
      onValueChange={(values) => {
        const next = Array.isArray(values) ? values[0] : values;
        if (typeof next === "string" && next !== "custom") onSelect(next);
      }}
      variant="outline"
      size="default"
      // The group ships `w-fit`, which overflows its container rather than
      // wrapping; capping it at the container is what lets the wrap engage.
      className="max-w-full min-w-0 flex-wrap"
    >
      {presets.map(({ token, label }) => (
        <ToggleGroupItem key={token} value={token}>
          {label}
        </ToggleGroupItem>
      ))}
      <Popover
        open={open}
        onOpenChange={(next) => {
          // Opening starts from what is selected now, not from the last draft.
          if (next) setPicked(undefined);
          setOpen(next);
        }}
      >
        <PopoverTrigger
          render={
            <ToggleGroupItem
              value="custom"
              className="gap-1.5"
              aria-label={
                custom ? rangeLabel(selected) : "Custom date range"
              }
            >
              <CalendarIcon className="size-3.5" />
              <span className="hidden sm:inline">
                {custom ? rangeLabel(selected) : "Custom"}
              </span>
            </ToggleGroupItem>
          }
        />
        <PopoverContent align="end" className="w-auto p-0">
          <p className="border-b border-border px-4 py-3 text-xs text-muted-foreground">
            The end date is not counted — pick the day after the last one you
            want.
          </p>
          <Calendar
            mode="range"
            resetOnSelect
            showOutsideDays={false}
            selected={showing}
            onSelect={(range) => setPicked(range)}
            defaultMonth={showing?.from}
            numberOfMonths={
              typeof window !== "undefined" && window.innerWidth < 640 ? 1 : 2
            }
          />
          <div className="flex items-center justify-end gap-3 border-t border-border px-4 py-2">
            <Button variant="ghost" size="sm" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button
              size="sm"
              disabled={!isWholeSpan(showing)}
              onClick={() => {
                if (!isWholeSpan(showing)) return;
                onSelect(
                  toRangeToken({
                    from: toISODate(showing.from),
                    to: toISODate(showing.to),
                  }),
                );
                setOpen(false);
              }}
            >
              Apply
            </Button>
          </div>
        </PopoverContent>
      </Popover>
    </ToggleGroup>
  );
}

/**
 * A span the server will accept: two days, the later one exclusive. One day
 * clicked twice is an empty window, so it is not one yet.
 */
function isWholeSpan(
  range: DayPickerRange | undefined,
): range is { from: Date; to: Date } {
  return Boolean(
    range?.from && range.to && toISODate(range.from) < toISODate(range.to),
  );
}
