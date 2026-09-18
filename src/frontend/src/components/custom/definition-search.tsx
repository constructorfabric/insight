import { Search } from "lucide-react";

import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

/**
 * A search box over a catalogue.
 *
 * The service matches the stored body as well as the name, so a table or a
 * column finds every definition that reads it — which is the question a
 * catalogue of a few hundred is actually asked.
 */
export function DefinitionSearch({
  label,
  value,
  onChange,
  className,
}: {
  label: string;
  value: string;
  onChange: (needle: string) => void;
  className?: string;
}) {
  return (
    <div className={cn("relative w-full max-w-64", className)}>
      <Search
        className="pointer-events-none absolute start-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
        aria-hidden
      />
      <Input
        type="search"
        value={value}
        aria-label={label}
        placeholder={label}
        className="ps-9"
        onChange={(event) => onChange(event.target.value)}
      />
    </div>
  );
}
