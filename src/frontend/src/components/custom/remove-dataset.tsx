import { ConfirmRemove } from "@/components/custom/confirm-remove";
import { Button } from "@/components/ui/button";
import { useRemoveDataset } from "@/queries/custom";

/**
 * Takes the dataset away, with the records it holds.
 *
 * The service refuses while a metric reads it and names every one, so the
 * refusal is shown as it came back.
 */
export function RemoveDataset({
  name,
  onRemoved,
}: {
  name: string;
  onRemoved: () => void;
}) {
  const remove = useRemoveDataset();

  return (
    <ConfirmRemove
      ask={(open) => (
        <Button
          variant="ghost"
          size="sm"
          aria-label={`Remove ${name}`}
          onClick={open}
        >
          Remove
        </Button>
      )}
      confirm="Remove it, with its records"
      pending={remove.isPending}
      error={remove.error}
      onRemove={() => remove.mutate(name, { onSuccess: onRemoved })}
      onKeep={remove.reset}
    />
  );
}
