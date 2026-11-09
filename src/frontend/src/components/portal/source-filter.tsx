import { useMemo, useState } from "react";
import { ListFilter, Search } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  Button,
  Checkbox,
  Input,
  Popover,
  PopoverContent,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from "@gears-frontx/ui-kit";

interface SourceFilterProps {
  sources: readonly string[];
  picked: readonly string[];
  failed?: boolean;
  onChange: (next: string[]) => void;
}

export function SourceFilter({
  sources,
  picked,
  failed = false,
  onChange,
}: SourceFilterProps) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const options = useMemo(
    () =>
      normalizedQuery
        ? sources.filter((source) =>
            source.toLocaleLowerCase().includes(normalizedQuery)
          )
        : sources,
    [sources, normalizedQuery]
  );
  const active = picked.length > 0;
  const label = t("identities.sources.title");

  const toggle = (source: string) => {
    onChange(
      picked.includes(source)
        ? picked.filter((entry) => entry !== source)
        : [...picked, source]
    );
  };

  return (
    <Popover>
      <PopoverTrigger
        render={
          <Button
            type="button"
            variant={active ? "secondary" : "outline"}
            size="sm"
            icon={<ListFilter />}
            aria-label={active ? `${label}: ${picked.join(", ")}` : undefined}
          >
            {label}
          </Button>
        }
      />
      <PopoverContent align="end" className="w-72 gap-3 p-3">
        <PopoverHeader className="flex-row items-center justify-between gap-2">
          <PopoverTitle>{label}</PopoverTitle>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => onChange([])}
          >
            {t("identities.sources.all")}
          </Button>
        </PopoverHeader>

        <div className="relative">
          <Search
            className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
            aria-hidden
          />
          <Input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t("identities.sources.search")}
            aria-label={t("identities.sources.search")}
            className="ps-8"
          />
        </div>

        <div className="max-h-64 space-y-1 overflow-y-auto">
          {options.map((source, index) => {
            const checkboxId = `source-filter-${index}`;

            return (
              <label
                key={source}
                htmlFor={checkboxId}
                className="flex cursor-pointer items-center gap-2 rounded-sm px-1 py-1.5 text-sm hover:bg-muted"
              >
                <Checkbox
                  id={checkboxId}
                  checked={picked.includes(source)}
                  onCheckedChange={() => toggle(source)}
                />
                <span className="min-w-0 truncate">{source}</span>
              </label>
            );
          })}
          {failed ? (
            <p
              role="alert"
              className="px-1 py-4 text-center text-sm text-destructive"
            >
              {t("identities.sources.failed")}
            </p>
          ) : null}
          {options.length === 0 && !failed ? (
            <p className="px-1 py-4 text-center text-sm text-muted-foreground">
              {t("identities.sources.none")}
            </p>
          ) : null}
        </div>
      </PopoverContent>
    </Popover>
  );
}
