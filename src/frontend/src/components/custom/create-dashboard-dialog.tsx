import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { MessageSquare, Plus, X, type LucideIcon } from "lucide-react";
import { useState } from "react";

import {
  createDefinition,
  DEFINITION_NAME_MAX,
  EVERYONE,
  putGrant,
  type DashboardVisibility,
  type Grantee,
} from "@/api/custom-client";
import type { PersonSummary } from "@/api/identity-client";
import { refusal } from "@/components/custom/refusal";
import { PersonPicker } from "@/components/portal/person-picker";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { toast } from "@/components/ui/sonner";
import { Spinner } from "@/components/ui/spinner";
import { askAssistant } from "@/lib/custom/assistant-draft";
import { personDisplayName } from "@/lib/identities/person-display";
import { useAssistantAvailable } from "@/lib/portal/use-assistant-available";
import { TEXT_BODY, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import { invalidateDashboardList } from "@/queries/custom";
import { useIsAdmin } from "@/queries/identity-me";

type Visibility = DashboardVisibility;

type StartFrom = "empty" | "assistant";

const VISIBILITIES = [
  { value: "private", label: "Private", caption: "Only you" },
  {
    value: "everyone",
    label: "Shared with org",
    caption: "Everyone can view · only you can edit",
  },
  {
    value: "people",
    label: "Shared with people",
    caption: "People you pick can view · only you can edit",
  },
] as const satisfies readonly {
  value: Visibility;
  label: string;
  caption: string;
}[];

const STARTS = [
  { value: "empty", label: "Empty dashboard", icon: Plus },
  { value: "assistant", label: "Describe it to the Assistant", icon: MessageSquare },
] as const satisfies readonly {
  value: StartFrom;
  label: string;
  icon: LucideIcon;
}[];

const NAME_FIELD_ID = "new-dashboard-name";

interface NewDashboard {
  title: string;
  grantees: Grantee[];
}

interface Created {
  id: string;
  shareFailure: unknown;
}

function granteesFor(
  visibility: Visibility,
  people: PersonSummary[]
): Grantee[] {
  switch (visibility) {
    case "private":
      return [];
    case "people":
      return people.map((person) => person.person_id);
    case "everyone":
      return [EVERYONE];
  }
}

function assistantPrompt(title: string): string {
  return `Build a dashboard named "${title}" that shows: `;
}

async function createShared({
  title,
  grantees,
}: NewDashboard): Promise<Created> {
  const id = await createDefinition("dashboards", title, { title, items: [] });

  const target = { kind: "dashboards", id } as const;
  const shares = await Promise.allSettled(
    grantees.map((grantee) => putGrant(target, grantee, "viewer"))
  );
  const refused = shares.find(
    (share): share is PromiseRejectedResult => share.status === "rejected"
  );

  return { id, shareFailure: refused?.reason ?? null };
}

export function NewDashboardButton() {
  const { isAdmin } = useIsAdmin();
  const [open, setOpen] = useState(false);

  if (!isAdmin) return null;

  return (
    <>
      <Button size="sm" icon={<Plus />} onClick={() => setOpen(true)}>
        New dashboard
      </Button>
      <CreateDashboardDialog open={open} onClose={() => setOpen(false)} />
    </>
  );
}

export function CreateDashboardDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const inDashboards = useRouterState({
    select: (state) => state.location.pathname.startsWith("/portal/custom"),
  });
  const assistantAvailable = useAssistantAvailable();
  const create = useMutation({
    mutationFn: createShared,
    onSettled: () => invalidateDashboardList(queryClient),
  });

  const [name, setName] = useState("");
  const [visibility, setVisibility] = useState<Visibility>("private");
  const [people, setPeople] = useState<PersonSummary[]>([]);
  const [start, setStart] = useState<StartFrom>("empty");
  const [error, setError] = useState<string | null>(null);

  const title = name.trim();
  const pending = create.isPending;
  const toAssistant = assistantAvailable && start === "assistant";
  const audienceChosen = visibility !== "people" || people.length > 0;
  const ready = title !== "" && (toAssistant || audienceChosen);

  function close() {
    setName("");
    setVisibility("private");
    setPeople([]);
    setStart("empty");
    setError(null);
    create.reset();
    onClose();
  }

  function requestClose() {
    if (pending) return;
    close();
  }

  function handToAssistant() {
    close();
    askAssistant(assistantPrompt(title));
    if (!inDashboards) void navigate({ to: "/portal/custom" });
  }

  async function submit() {
    if (!ready || pending) return;
    if (toAssistant) {
      handToAssistant();
      return;
    }

    setError(null);
    try {
      const made = await create.mutateAsync({
        title,
        grantees: granteesFor(visibility, people),
      });
      close();
      if (made.shareFailure !== null) {
        toast.error(
          refusal(made.shareFailure, "Created, but couldn't share it.")
        );
      }
      void navigate({ to: "/portal/custom/$name", params: { name: made.id } });
    } catch (failure) {
      setError(refusal(failure, "Couldn't create the dashboard."));
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) requestClose();
      }}
    >
      <DialogContent className="sm:max-w-[520px]" showCloseButton={!pending}>
        <DialogHeader>
          <DialogTitle>New dashboard</DialogTitle>
        </DialogHeader>

        <div className="flex flex-col gap-5">
          <div className="flex flex-col gap-2">
            <Label htmlFor={NAME_FIELD_ID}>Name</Label>
            <Input
              id={NAME_FIELD_ID}
              autoFocus
              required
              maxLength={DEFINITION_NAME_MAX}
              readOnly={pending}
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setError(null);
              }}
              onKeyDown={(event) => {
                if (event.key !== "Enter") return;
                event.preventDefault();
                void submit();
              }}
            />
          </div>

          <fieldset
            disabled={toAssistant || pending}
            className="flex min-w-0 flex-col gap-2"
          >
            <legend className={cn(TEXT_LABEL, "mb-2 font-medium")}>
              Visibility
            </legend>
            <VisibilityChoices value={visibility} onChange={setVisibility} />
            {visibility === "people" ? (
              <NamedPeople
                people={people}
                onAdd={(person) => setPeople((was) => [...was, person])}
                onRemove={(personId) =>
                  setPeople((was) =>
                    was.filter((person) => person.person_id !== personId)
                  )
                }
              />
            ) : null}
          </fieldset>

          {assistantAvailable ? (
            <fieldset
              disabled={pending}
              className="flex min-w-0 flex-col gap-2"
            >
              <legend className={cn(TEXT_LABEL, "mb-2 font-medium")}>
                Start from
              </legend>
              <StartChoices value={start} onChange={setStart} />
            </fieldset>
          ) : null}

          {error ? (
            <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
              {error}
            </p>
          ) : null}

          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              disabled={pending}
              onClick={requestClose}
            >
              Cancel
            </Button>
            <Button
              type="button"
              disabled={!ready || pending}
              onClick={() => void submit()}
            >
              {pending ? <Spinner className="size-3" /> : null}
              Create
            </Button>
          </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function VisibilityChoices({
  value,
  onChange,
}: {
  value: Visibility;
  onChange: (next: Visibility) => void;
}) {
  return (
    <div className="flex flex-col gap-3">
      {VISIBILITIES.map((option) => (
        <label
          key={option.value}
          className="flex w-fit cursor-pointer items-start gap-2"
        >
          <input
            type="radio"
            name="new-dashboard-visibility"
            value={option.value}
            checked={value === option.value}
            className="mt-0.5 size-4 accent-primary"
            onChange={() => onChange(option.value)}
          />
          <span className="flex flex-col">
            <span className={cn(TEXT_BODY, "font-medium")}>{option.label}</span>
            <span className={cn(TEXT_LABEL, "font-normal text-muted-foreground")}>
              {option.caption}
            </span>
          </span>
        </label>
      ))}
    </div>
  );
}

function StartChoices({
  value,
  onChange,
}: {
  value: StartFrom;
  onChange: (next: StartFrom) => void;
}) {
  return (
    <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
      {STARTS.map(({ value: option, label, icon: Icon }) => {
        const picked = value === option;

        return (
          <label
            key={option}
            className={cn(
              "flex cursor-pointer flex-col gap-2 rounded-lg border p-3 transition-colors has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring",
              picked
                ? "border-primary bg-primary/10"
                : "border-border bg-card hover:bg-muted"
            )}
          >
            <input
              type="radio"
              name="new-dashboard-start"
              value={option}
              checked={picked}
              className="sr-only"
              onChange={() => onChange(option)}
            />
            <Icon className="size-5 text-muted-foreground" aria-hidden />
            <span className={cn(TEXT_BODY, "font-medium")}>{label}</span>
          </label>
        );
      })}
    </div>
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
