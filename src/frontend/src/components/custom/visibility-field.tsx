import { X } from "lucide-react";
import type { Dispatch, SetStateAction } from "react";

import type { DashboardVisibility } from "@/api/custom-client";
import type { PersonSummary } from "@/api/identity-client";
import { PersonPicker } from "@/components/portal/person-picker";
import { Button } from "@/components/ui/button";
import { personDisplayName } from "@/lib/identities/person-display";
import { TEXT_BODY, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

const LABELS = {
  private: "Private",
  everyone: "Shared with org",
  people: "Shared with people",
} as const satisfies Record<DashboardVisibility, string>;

const ORDER = [
  "private",
  "everyone",
  "people",
] as const satisfies readonly DashboardVisibility[];

export function VisibilityField({
  name,
  captions,
  value,
  people,
  disabled,
  onChange,
  onPeopleChange,
}: {
  name: string;
  captions: Record<DashboardVisibility, string>;
  value: DashboardVisibility;
  people: PersonSummary[];
  disabled: boolean;
  onChange: (next: DashboardVisibility) => void;
  onPeopleChange: Dispatch<SetStateAction<PersonSummary[]>>;
}) {
  return (
    <fieldset disabled={disabled} className="flex min-w-0 flex-col gap-2">
      <legend className={cn(TEXT_LABEL, "mb-2 font-medium")}>Visibility</legend>
      <div className="flex flex-col gap-3">
        {ORDER.map((option) => (
          <label
            key={option}
            className="flex w-fit cursor-pointer items-start gap-2"
          >
            <input
              type="radio"
              name={name}
              value={option}
              checked={value === option}
              className="mt-0.5 size-4 accent-primary"
              onChange={() => onChange(option)}
            />
            <span className="flex flex-col">
              <span className={cn(TEXT_BODY, "font-medium")}>
                {LABELS[option]}
              </span>
              <span
                className={cn(TEXT_LABEL, "font-normal text-muted-foreground")}
              >
                {captions[option]}
              </span>
            </span>
          </label>
        ))}
      </div>
      {value === "people" ? (
        <NamedPeople
          people={people}
          onAdd={(person) => onPeopleChange((was) => [...was, person])}
          onRemove={(personId) =>
            onPeopleChange((was) =>
              was.filter((person) => person.person_id !== personId)
            )
          }
        />
      ) : null}
    </fieldset>
  );
}

function NamedPeople({
  people,
  onAdd,
  onRemove,
}: {
  people: PersonSummary[];
  onAdd: (person: PersonSummary) => void;
  onRemove: (personId: string) => void;
}) {
  return (
    <div className="flex flex-col gap-2 ps-6">
      {people.length > 0 ? (
        <ul aria-label="Named people" className="flex flex-col gap-1">
          {people.map((person) => {
            const shown = personDisplayName(person);

            return (
              <li key={person.person_id} className="flex items-center gap-2">
                <span className={cn(TEXT_BODY, "min-w-0 flex-1 truncate")}>
                  {shown}
                </span>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label={`Remove ${shown}`}
                  icon={<X />}
                  onClick={() => onRemove(person.person_id)}
                />
              </li>
            );
          })}
        </ul>
      ) : null}
      <PersonPicker
        excludeIds={people.map((person) => person.person_id)}
        onPick={onAdd}
      />
    </div>
  );
}
