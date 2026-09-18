import { Trash2 } from "lucide-react";

import type { DefinitionKind } from "@/api/custom-client";
import { ConfirmRemove } from "@/components/custom/confirm-remove";
import { Button } from "@/components/ui/button";
import { useRemoveDefinition } from "@/queries/custom";

/**
 * Removes one definition.
 *
 * The service refuses while something still draws it and names what does, so
 * the refusal is shown as it came back rather than guessed at here.
 */
export function RemoveDefinition({
  kind,
  name,
  onRemoved,
}: {
  kind: DefinitionKind;
  name: string;
  onRemoved?: () => void;
}) {
  const remove = useRemoveDefinition();

  return (
    <ConfirmRemove
      ask={(open) => (
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`Remove ${name}`}
          className="text-muted-foreground hover:text-destructive"
          onClick={open}
        >
          <Trash2 />
        </Button>
      )}
      confirm="Remove"
      pending={remove.isPending}
      error={remove.error}
      onRemove={() => remove.mutate({ kind, name }, { onSuccess: onRemoved })}
      onKeep={remove.reset}
    />
  );
}
