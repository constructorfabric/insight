import { useState } from "react";
import { Pencil } from "lucide-react";

import type { DefinitionKind } from "@/api/custom-client";
import { refusal } from "@/components/custom/refusal";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { useRenameDefinition } from "@/queries/custom";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/**
 * Renames one definition.
 *
 * A name is the only handle a widget has on its metric, and a dashboard on its
 * widgets, so the service rewrites whatever pointed at the old name rather
 * than leaving the reader to fix them one at a time. A name already taken is
 * refused, and the refusal is shown as it came back.
 */
export function RenameDefinition({
  kind,
  name,
}: {
  kind: DefinitionKind;
  name: string;
}) {
  const [asked, setAsked] = useState(false);
  const [next, setNext] = useState(name);
  const rename = useRenameDefinition();

  if (!asked) {
    return (
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={`Rename ${name}`}
        className="text-muted-foreground"
        onClick={() => {
          setNext(name);
          setAsked(true);
        }}
      >
        <Pencil />
      </Button>
    );
  }

  const submit = () => {
    const to = next.trim();
    if (!to || to === name) {
      setAsked(false);
      return;
    }

    rename.mutate({ kind, name, to }, { onSuccess: () => setAsked(false) });
  };

  return (
    <span className="flex flex-col items-end gap-1">
      <span className="flex items-center gap-1">
        <Input
          value={next}
          aria-label={`New name for ${name}`}
          autoFocus
          className="h-7 w-56 font-mono"
          onChange={(event) => setNext(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") submit();
            if (event.key === "Escape") setAsked(false);
          }}
        />
        <Button
          variant="ghost"
          size="sm"
          disabled={rename.isPending}
          onClick={submit}
        >
          {rename.isPending ? <Spinner className="size-3" /> : null}
          Rename
        </Button>
        <Button variant="ghost" size="sm" onClick={() => setAsked(false)}>
          Cancel
        </Button>
      </span>

      {rename.isError ? (
        <span role="alert" className={cn(TEXT_LABEL, "text-destructive")}>
          {refusal(rename.error, "Couldn't rename it.")}
        </span>
      ) : null}
    </span>
  );
}
