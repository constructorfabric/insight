import { useId, useState } from "react";
import { Search, Users } from "lucide-react";

import type { MembersGridMember } from "@/components/widgets/dashboard/members-grid";
import {
  Button,
  Checkbox,
  Input,
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@gears-frontx/ui-kit";
import type { CompareTeam } from "@/lib/portal/metrics-view";
import { TEXT_EYEBROW } from "@/lib/type-scale";

export function MetricsComparePicker({
  people,
  teams,
  picked,
  onCompare,
}: {
  people: readonly MembersGridMember[];
  teams: readonly CompareTeam[];
  picked: readonly string[];
  onCompare: (entityIds: string[]) => void;
}) {
  const idPrefix = useId();
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<string[]>([]);
  const [query, setQuery] = useState("");

  const needle = query.trim().toLocaleLowerCase();
  const matches = (name: string) => !needle || name.toLocaleLowerCase().includes(needle);
  const sortedPeople = [...people].sort((a, b) => a.displayName.localeCompare(b.displayName));

  const toggle = (entityId: string) =>
    setDraft((current) =>
      current.includes(entityId)
        ? current.filter((id) => id !== entityId)
        : [...current, entityId],
    );
  const teamPicked = (team: CompareTeam) =>
    team.memberIds.every((id) => draft.includes(id));
  const toggleTeam = (team: CompareTeam) =>
    setDraft((current) =>
      team.memberIds.every((id) => current.includes(id))
        ? current.filter((id) => !team.memberIds.includes(id))
        : [...new Set([...current, ...team.memberIds])],
    );

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        if (next) {
          setDraft([...picked]);
          setQuery("");
        }
        setOpen(next);
      }}
    >
      <PopoverTrigger
        render={
          <Button type="button" variant="outline" size="sm" icon={<Users />}>
            Compare
          </Button>
        }
      />
      <PopoverContent align="end" className="w-80 gap-3 p-3">
        <div className="relative">
          <Search
            className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
            aria-hidden
          />
          <Input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Find a person or a team"
            aria-label="Find a person or a team"
            className="ps-8"
          />
        </div>

        <div className="flex max-h-80 flex-col gap-3 overflow-y-auto">
          {teams.some((team) => matches(team.name)) ? (
            <section className="flex flex-col gap-1">
              <h3 className={TEXT_EYEBROW}>Teams</h3>
              {teams
                .filter((team) => matches(team.name))
                .map((team) => {
                  const checkboxId = `${idPrefix}-team-${team.id}`;
                  return (
                    <label
                      key={team.id}
                      htmlFor={checkboxId}
                      className="flex cursor-pointer items-center gap-2 rounded-sm px-1 py-1.5 text-sm hover:bg-muted"
                    >
                      <Checkbox
                        id={checkboxId}
                        checked={teamPicked(team)}
                        onCheckedChange={() => toggleTeam(team)}
                      />
                      <span className="min-w-0 flex-1 truncate">{`${team.name}'s team`}</span>
                      <span aria-hidden className="text-xs text-muted-foreground tabular-nums">
                        {team.memberIds.length}
                      </span>
                    </label>
                  );
                })}
            </section>
          ) : null}

          <section className="flex flex-col gap-1">
            <h3 className={TEXT_EYEBROW}>People</h3>
            {sortedPeople
              .filter((person) => matches(person.displayName))
              .map((person, index) => {
                const checkboxId = `${idPrefix}-${index}`;
                return (
                  <label
                    key={person.entityId}
                    htmlFor={checkboxId}
                    className="flex cursor-pointer items-center gap-2 rounded-sm px-1 py-1.5 text-sm hover:bg-muted"
                  >
                    <Checkbox
                      id={checkboxId}
                      checked={draft.includes(person.entityId)}
                      onCheckedChange={() => toggle(person.entityId)}
                    />
                    <span className="min-w-0 truncate">{person.displayName}</span>
                  </label>
                );
              })}
          </section>
        </div>

        <div className="flex items-center justify-between gap-2 border-t pt-3">
          <Button type="button" variant="ghost" size="sm" onClick={() => setDraft([])}>
            Clear
          </Button>
          <Button
            type="button"
            size="sm"
            disabled={draft.length < 2}
            onClick={() => {
              onCompare(draft);
              setOpen(false);
            }}
          >
            {`Compare (${draft.length})`}
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
