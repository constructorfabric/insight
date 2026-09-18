import { useState } from "react";
import { Trash2 } from "lucide-react";

import type { DefinitionKind } from "@/api/custom-client";
import { refusal } from "@/components/custom/refusal";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { useRemoveDefinition } from "@/queries/custom";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/**
 * Removes one definition, in two clicks.
 *
 * The service refuses while something still draws it and names what does, so
 * the refusal is shown as it came back rather than guessed at here.
 */
export function RemoveDefinition({
  kind,
  name,
}: {
  kind: DefinitionKind;
  name: string;
}) {
  const [asked, setAsked] = useState(false);
  const remove = useRemoveDefinition();

  if (remove.isError) {
    return (
      <span role="alert" className={cn(TEXT_LABEL, "text-destructive")}>
        {refusal(remove.error, "Couldn't remove it.")}
      </span>
    );
  }

  if (!asked) {
    return (
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={`Remove ${name}`}
        className="text-muted-foreground hover:text-destructive"
        onClick={() => setAsked(true)}
      >
        <Trash2 />
      </Button>
    );
  }

  return (
    <span className="flex items-center gap-1">
      <Button
        variant="ghost"
        size="sm"
        className="text-destructive"
        disabled={remove.isPending}
        onClick={() => remove.mutate({ kind, name })}
      >
        {remove.isPending ? <Spinner className="size-3" /> : null}
        Remove
      </Button>
      <Button variant="ghost" size="sm" onClick={() => setAsked(false)}>
        Keep
      </Button>
    </span>
  );
}
