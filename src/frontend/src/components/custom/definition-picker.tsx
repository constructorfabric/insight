import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useRef, useState } from "react";

import type { DefinitionKind } from "@/api/custom-client";
import {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
} from "@/components/ui/combobox";
import { refusal } from "@/components/custom/refusal";
import { useAutoLoadOnScroll } from "@/hooks/use-auto-load-on-scroll";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import type { Describing } from "@/lib/custom/editor/aria";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import {
  dashboardReadQuery,
  definitionPagesQuery,
  metricQuery,
  widgetQuery,
} from "@/queries/custom";

/** Long enough that a word is typed before the service is asked. */
const SEARCH_DEBOUNCE_MS = 300;

const NOUN: Record<DefinitionKind, { one: string; many: string }> = {
  metrics: { one: "metric", many: "metrics" },
  widgets: { one: "widget", many: "widgets" },
  dashboards: { one: "dashboard", many: "dashboards" },
};

/**
 * Picks one stored definition, by id, shown by its name.
 *
 * The service does the searching — it matches the stored body as well as the
 * name, so a table name finds every metric that reads it — and the list reads
 * its matches a page at a time as it is scrolled.
 */
export function DefinitionPicker({
  kind,
  id,
  value,
  describe,
  onChange,
}: {
  kind: DefinitionKind;
  id: string;
  value: string;
  describe?: Describing;
  onChange: (id: string) => void;
}) {
  const noun = NOUN[kind];
  const [typed, setTyped] = useState("");
  const searching = useDebouncedValue(typed, SEARCH_DEBOUNCE_MS).trim();
  const found = useInfiniteQuery(definitionPagesQuery(kind, searching));
  const chosenName = useStoredName(kind, value);
  // INVARIANT: pages are read by offset, so a definition saved between two
  // reads shifts the next page and repeats one; each id is one option.
  const labels = new Map(
    found.data?.pages.flatMap((page) =>
      page.items.map((item) => [item.id, item.name] as const)
    )
  );
  const ids = [...labels.keys()];
  const labelOf = (option: string) =>
    labels.get(option) ?? (option === value ? chosenName : undefined) ?? option;
  const list = useRef<HTMLDivElement>(null);
  const marker = useAutoLoadOnScroll({
    hasNextPage: found.hasNextPage,
    isFetchingNextPage: found.isFetchingNextPage,
    fetchNextPage: () => void found.fetchNextPage(),
    root: list,
  });

  return (
    <Combobox<string>
      items={ids}
      itemToStringLabel={labelOf}
      filter={null}
      value={value === "" ? null : value}
      onValueChange={(next) => onChange(next ?? "")}
      onInputValueChange={setTyped}
    >
      <ComboboxInput
        id={id}
        {...describe}
        placeholder={`Search ${noun.many}`}
        className="h-9 w-full"
      />
      <ComboboxContent>
        {/* WORKAROUND: the kit pads the empty part even while it hides its
            text, which leaves a gap above a full list. */}
        {ids.length === 0 ? (
          <ComboboxEmpty>
            {found.isError ? (
              <>
                {refusal(found.error, `Couldn't load ${noun.many}.`)}{" "}
                <button
                  type="button"
                  className="underline"
                  onClick={() => void found.refetch()}
                >
                  Retry
                </button>
              </>
            ) : found.isPending ? (
              "Searching…"
            ) : (
              `No ${noun.one} matches.`
            )}
          </ComboboxEmpty>
        ) : null}
        <ComboboxList ref={list}>
          {ids.map((option) => (
            <ComboboxItem key={option} value={option}>
              {labelOf(option)}
            </ComboboxItem>
          ))}
          {found.isFetchNextPageError ? (
            <p
              role="alert"
              className={cn(TEXT_LABEL, "px-3 py-2 text-destructive")}
            >
              {refusal(found.error, `Couldn't load more ${noun.many}.`)}{" "}
              <button
                type="button"
                className="underline"
                onClick={() => void found.fetchNextPage()}
              >
                Retry
              </button>
            </p>
          ) : found.hasNextPage ? (
            <p
              ref={marker}
              role="presentation"
              aria-hidden={!found.isFetchingNextPage}
              className={cn(TEXT_LABEL, "px-3 py-2")}
            >
              {found.isFetchingNextPage ? "Loading more…" : " "}
            </p>
          ) : null}
        </ComboboxList>
      </ComboboxContent>
    </Combobox>
  );
}

/** The chosen definition's name, read through the query its own page uses. */
function useStoredName(kind: DefinitionKind, id: string): string | undefined {
  const asked = id !== "";
  const metric = useQuery({
    ...metricQuery(id),
    enabled: asked && kind === "metrics",
  });
  const widget = useQuery({
    ...widgetQuery(id),
    enabled: asked && kind === "widgets",
  });
  const dashboard = useQuery({
    ...dashboardReadQuery(id),
    enabled: asked && kind === "dashboards",
  });

  switch (kind) {
    case "metrics":
      return metric.data?.name;
    case "widgets":
      return widget.data?.name;
    case "dashboards":
      return dashboard.data?.name;
  }
}
