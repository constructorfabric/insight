import { refusal } from "@/components/custom/refusal";
import { isRevisionConflict } from "@/lib/alerts/conflict";
import { Switch } from "@/components/ui/switch";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import { useSetAlertEnabled } from "@/queries/alerts";

/**
 * Turns an alert's checks on or off where it is listed or shown.
 *
 * `revision` is passed where the caller holds the alert; the list holds only a
 * summary, and the alert is then read before the change is sent.
 */
export function AlertSwitch({
  id,
  name,
  enabled,
  revision,
}: {
  id: string;
  name: string;
  enabled: boolean;
  revision?: number;
}) {
  const toggle = useSetAlertEnabled();
  const shown = toggle.isPending ? toggle.variables.enabled : enabled;

  return (
    <span className="inline-flex flex-col items-end gap-1">
      <span className="inline-flex items-center gap-2">
        <span className={TEXT_LABEL} aria-hidden>
          {shown ? "On" : "Off"}
        </span>
        <Switch
          checked={shown}
          aria-label={`Checks for ${name}`}
          aria-busy={toggle.isPending || undefined}
          // WORKAROUND: not disabled while saving, since a browser drops focus from a disabled control.
          onCheckedChange={(next: boolean) => {
            if (!toggle.isPending)
              toggle.mutate({ id, enabled: next, revision });
          }}
        />
      </span>
      {toggle.isError ? (
        <span role="alert" className={cn(TEXT_LABEL, "text-destructive")}>
          {isRevisionConflict(toggle.error)
            ? "It changed elsewhere. Try again."
            : refusal(toggle.error, "Couldn't change it.")}
        </span>
      ) : null}
    </span>
  );
}
