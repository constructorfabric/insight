import { useInfiniteQuery } from "@tanstack/react-query";
import { useState } from "react";

import {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
} from "@/components/ui/combobox";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import type { Describing } from "@/lib/custom/editor/aria";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import { definitionPagesQuery } from "@/queries/custom";

/** Long enough that a word is typed before the service is asked. */
const SEARCH_DEBOUNCE_MS = 300;

/**
 * Picks one stored metric.
 *
 * The service does the searching — it matches the stored body as well as the
 * name, so a table name finds every metric that reads it — and the list shows
 * its first page of matches, so any number of metrics can be reached by typing.
 */
export function MetricPicker({
  id,
  value,
  describe,
  onChange,
}: {
  id: string;
  value: string;
  describe?: Describing;
  onChange: (metric: string) => void;
}) {
  const [typed, setTyped] = useState("");
  const searching = useDebouncedValue(typed, SEARCH_DEBOUNCE_MS).trim();
  const found = useInfiniteQuery(definitionPagesQuery("metrics", searching));
  const first = found.data?.pages[0];
  const names = first?.names ?? [];
  const more = first ? first.total - names.length : 0;

  return (
    <Combobox<string>
      items={names}
      filter={null}
      value={value === "" ? null : value}
      onValueChange={(next) => onChange(next ?? "")}
      onInputValueChange={setTyped}
    >
      <ComboboxInput
        id={id}
        {...describe}
        placeholder="Search metrics"
        className="h-9 w-full font-mono"
      />
      <ComboboxContent>
        {/* WORKAROUND: the kit pads the empty part even while it hides its text, which leaves a gap above a full list. */}
        {names.length === 0 ? (
          <ComboboxEmpty>
            {found.isPending ? "Searching…" : "No metric matches."}
          </ComboboxEmpty>
        ) : null}
        <ComboboxList>
          {(name: string) => (
            <ComboboxItem key={name} value={name} className="font-mono">
              {name}
            </ComboboxItem>
          )}
        </ComboboxList>
        {more > 0 ? (
          <p className={cn(TEXT_LABEL, "px-3 py-2")}>
            {more} more match. Keep typing to narrow the list.
          </p>
        ) : null}
      </ComboboxContent>
    </Combobox>
  );
}
