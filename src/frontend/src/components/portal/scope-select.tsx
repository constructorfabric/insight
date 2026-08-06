import { Check, ChevronDown } from "lucide-react";

import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Switch } from "@/components/ui/switch";
import {
  usePortalNavActions,
  usePortalScope,
} from "@/lib/portal/portal-nav";
import { useOrgScope } from "@/lib/portal/use-org-scope";
import { cn } from "@/lib/utils";

/**
 * Global org-scope control (design §6): pick any manager node of the viewer's
 * subtree; optionally narrow to direct reports. The active scope is the frame
 * every org zone (Overview / Directions / AI & Cost / People) computes in.
 *
 * Options carry person ids since the identity cutover — the same key the routes
 * and the metric requests use. `pivotPersonId` and `managerNodes[].person_id`
 * come from one identity tree walk in `useOrgScope`, so a plain `===` is safe.
 */
export function ScopeSelect() {
  const { setScope } = usePortalNavActions();
  const scope = usePortalScope();
  const { label, count, managerNodes, pivotPersonId, canDirectOnly } = useOrgScope();
  if (!managerNodes.length) return null;

  return (
    <Popover>
      <PopoverTrigger
        render={
          <button
            type="button"
            className="flex h-9 items-center gap-1.5 rounded-md border bg-background px-3 text-sm shadow-xs hover:bg-accent"
          >
            {/* Narrow screens keep only the identity of the scope: the word
                "Scope" and the head-count are recoverable from the popover, and
                the sticky bar has ~300px for three controls. */}
            <span className="hidden text-muted-foreground md:inline">Scope:</span>
            <span className="max-w-28 truncate font-medium md:max-w-40">{label}</span>
            <span className="hidden text-muted-foreground md:inline">· {count}</span>
            <ChevronDown className="size-4 text-muted-foreground" aria-hidden />
          </button>
        }
      />
      <PopoverContent align="end" className="w-72 p-1">
        <div className="max-h-80 overflow-y-auto">
          {managerNodes.map((m) => (
            <button
              key={m.person_id}
              type="button"
              onClick={() => setScope({ root: m.depth === 0 ? null : m.person_id })}
              className={cn(
                "flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-sm hover:bg-accent",
                m.person_id === pivotPersonId && "bg-accent/60",
              )}
              style={{ paddingLeft: `${0.5 + m.depth * 0.875}rem` }}
            >
              <span className="min-w-0 flex-1 truncate">{m.name}</span>
              <span className="text-xs text-muted-foreground">{m.teamSize}</span>
              {m.person_id === pivotPersonId ? <Check className="size-4" aria-hidden /> : null}
            </button>
          ))}
        </div>
        {canDirectOnly ? (
          <label className="flex cursor-pointer items-center justify-between gap-2 border-t px-2 py-2 text-sm select-none">
            <span>Direct reports only</span>
            <Switch
              checked={scope.directOnly}
              onCheckedChange={(v) => setScope({ directOnly: v })}
            />
          </label>
        ) : null}
      </PopoverContent>
    </Popover>
  );
}
