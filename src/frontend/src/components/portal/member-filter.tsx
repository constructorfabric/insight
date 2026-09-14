import { useMemo, useState } from "react";
import { ListFilter, Search } from "lucide-react";

import type { MembersGridMember } from "@/components/widgets/dashboard/members-grid";
import {
  selectedMembers,
  type MemberSelection,
} from "@/components/portal/member-selection";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/popover";

interface MemberFilterProps {
  members: readonly MembersGridMember[];
  selection: MemberSelection;
  onChange: (selection: MemberSelection) => void;
}

export function MemberFilter({
  members,
  selection,
  onChange,
}: MemberFilterProps) {
  const [query, setQuery] = useState("");
  const selected = selectedMembers(members, selection);
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const options = useMemo(
    () =>
      normalizedQuery
        ? members.filter((member) =>
            member.displayName.toLocaleLowerCase().includes(normalizedQuery),
          )
        : members,
    [members, normalizedQuery],
  );
  const allSelected = selection.kind === "all";
  const label = allSelected
    ? "All people"
    : `${selected.length} of ${members.length} people`;
  const accessibleLabel = allSelected
    ? `Filter people, all ${members.length} shown`
    : `Filter people, ${selected.length} of ${members.length} shown`;

  const toggle = (entityId: string) => {
    const ids = new Set(
      selection.kind === "all"
        ? members.map((member) => member.entityId)
        : selection.ids,
    );
    if (!ids.delete(entityId)) ids.add(entityId);
    onChange({ kind: "selected", ids });
  };

  return (
    <Popover>
      <PopoverTrigger
        render={
          <Button
            type="button"
            variant="outline"
            size="sm"
            icon={<ListFilter />}
            aria-label={accessibleLabel}
          >
            {label}
          </Button>
        }
      />
      <PopoverContent align="end" className="w-72 gap-3 p-3">
        <PopoverHeader className="flex-row items-center justify-between gap-2">
          <PopoverTitle>People</PopoverTitle>
          <div className="flex items-center gap-1">
            <Button
              type="button"
              variant="ghost"
              size="xs"
              onClick={() => onChange({ kind: "all" })}
            >
              All
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="xs"
              onClick={() =>
                onChange({ kind: "selected", ids: new Set<string>() })
              }
            >
              None
            </Button>
          </div>
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
            placeholder="Find someone"
            aria-label="Find someone"
            className="ps-8"
          />
        </div>

        <div className="max-h-64 space-y-1 overflow-y-auto">
          {options.map((member, index) => {
            const checked =
              selection.kind === "all" || selection.ids.has(member.entityId);
            const checkboxId = `member-filter-${index}`;

            return (
              <label
                key={member.entityId}
                htmlFor={checkboxId}
                className="flex cursor-pointer items-center gap-2 rounded-sm px-1 py-1.5 text-sm hover:bg-muted"
              >
                <Checkbox
                  id={checkboxId}
                  checked={checked}
                  onCheckedChange={() => toggle(member.entityId)}
                />
                <span className="min-w-0 truncate">{member.displayName}</span>
              </label>
            );
          })}
          {options.length === 0 ? (
            <p className="px-1 py-4 text-center text-sm text-muted-foreground">
              No people found.
            </p>
          ) : null}
        </div>
      </PopoverContent>
    </Popover>
  );
}
