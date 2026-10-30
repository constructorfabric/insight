import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { MessageSquare, Plus, type LucideIcon } from "lucide-react";
import { useState } from "react";

import {
  createDefinition,
  DEFINITION_NAME_MAX,
  type DashboardVisibility,
  type Grantee,
} from "@/api/custom-client";
import type { PersonSummary } from "@/api/identity-client";
import { refusal } from "@/components/custom/refusal";
import { VisibilityField } from "@/components/custom/visibility-field";
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
import { audienceChosen, granteesFor } from "@/lib/custom/initial-grants";
import { useAssistantAvailable } from "@/lib/portal/use-assistant-available";
import { TEXT_BODY, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import { grantViewers, invalidateDashboardList } from "@/queries/custom";
import { useIsAdmin } from "@/queries/identity-me";

type Visibility = DashboardVisibility;

type StartFrom = "empty" | "assistant";

const CAPTIONS = {
  private: "Only you",
  everyone: "Everyone can view · only you can edit",
  people: "People you pick can view · only you can edit",
} as const satisfies Record<Visibility, string>;

const STARTS = [
  { value: "empty", label: "Empty dashboard", icon: Plus },
  { value: "assistant", label: "Describe it to the Assistant", icon: MessageSquare },
] as const satisfies readonly {
  value: StartFrom;
  label: string;
  icon: LucideIcon;
}[];

const NAME_FIELD_ID = "new-dashboard-name";
const ERROR_ID = "new-dashboard-error";

interface NewDashboard {
  title: string;
  grantees: Grantee[];
}

interface Created {
  id: string;
  shareFailure: { refused: unknown } | null;
}

function assistantPrompt(title: string): string {
  return `Build a dashboard named "${title}" that shows: `;
}

async function createShared({
  title,
  grantees,
}: NewDashboard): Promise<Created> {
  const id = await createDefinition("dashboards", title, { title, items: [] });

  const shareFailure = await grantViewers({ kind: "dashboards", id }, grantees);

  return { id, shareFailure };
}

export function NewDashboardButton({
  variant = "default",
}: {
  variant?: "default" | "outline";
}) {
  const { isAdmin } = useIsAdmin();
  const [open, setOpen] = useState(false);

  if (!isAdmin) return null;

  return (
    <>
      <Button
        variant={variant}
        size="sm"
        icon={<Plus />}
        onClick={() => setOpen(true)}
      >
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
  const ready =
    title !== "" && (toAssistant || audienceChosen(visibility, people));

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
          refusal(made.shareFailure.refused, "Created, but couldn't share it.")
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
              aria-invalid={error !== null}
              aria-describedby={error ? ERROR_ID : undefined}
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

          <VisibilityField
            name="new-dashboard-visibility"
            captions={CAPTIONS}
            value={visibility}
            people={people}
            disabled={toAssistant || pending}
            onChange={setVisibility}
            onPeopleChange={setPeople}
          />

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
            <p
              id={ERROR_ID}
              role="alert"
              className={cn(TEXT_BODY, "text-destructive")}
            >
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
