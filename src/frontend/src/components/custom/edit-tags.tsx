import { useQuery } from "@tanstack/react-query";
import { useId, useState } from "react";

import { DASHBOARD_TAGS_MAX, TAG_NAME_MAX } from "@/api/custom-client";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { refusal } from "@/components/custom/refusal";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { sameTagName } from "@/lib/custom/tag-names";
import {
  dashboardTagsQuery,
  tagsQuery,
  useSetDashboardTags,
} from "@/queries/custom";

const TOO_MANY = `A dashboard carries at most ${DASHBOARD_TAGS_MAX} tags.`;
const TOO_LONG = `A tag name is at most ${TAG_NAME_MAX} characters.`;

export function EditTags({
  name,
  open,
  onClose,
}: {
  name: string;
  open: boolean;
  onClose: () => void;
}) {
  const every = useQuery({ ...tagsQuery(), enabled: open });
  const carried = useQuery({
    ...dashboardTagsQuery(name),
    enabled: open,
    staleTime: 0,
  });
  const ready = carried.data !== undefined && !carried.isFetching;
  const save = useSetDashboardTags();
  const [ticked, setTicked] = useState<string[] | null>(null);
  const [added, setAdded] = useState<string[]>([]);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);

  const chosen = ticked ?? carried.data ?? [];
  const choices = [
    ...new Set([
      ...(every.data?.tags.map((tag) => tag.name) ?? []),
      ...(carried.data ?? []),
      ...added,
    ]),
  ];

  function close() {
    setTicked(null);
    setAdded([]);
    setDraft("");
    setError(null);
    save.reset();
    onClose();
  }

  function refuse(why: string): null {
    setError(why);
    return null;
  }

  function withTag(tag: string): string[] | null {
    if (chosen.includes(tag)) return chosen;
    if (chosen.length >= DASHBOARD_TAGS_MAX) return refuse(TOO_MANY);

    return [...chosen, tag];
  }

  function toggle(tag: string) {
    const next = chosen.includes(tag)
      ? chosen.filter((one) => one !== tag)
      : withTag(tag);
    if (!next) return;

    setTicked(next);
    setError(null);
  }

  function addDraft(): string[] | null {
    const typed = draft.trim();
    if (typed === "") return chosen;
    if ([...typed].length > TAG_NAME_MAX) return refuse(TOO_LONG);

    const tag = choices.find((one) => sameTagName(one, typed)) ?? typed;
    const next = withTag(tag);
    if (!next) return null;

    if (!choices.includes(tag)) setAdded([...added, tag]);
    setTicked(next);
    setDraft("");
    setError(null);

    return next;
  }

  function done() {
    const tags = addDraft();
    if (!tags) return;

    save.mutate(
      { name, tags },
      {
        onSuccess: close,
        onError: (failure) =>
          setError(refusal(failure, "The tags could not be saved.")),
      }
    );
  }

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
      title="Edit tags"
      description={`Tick the tags ${name} carries, up to ${DASHBOARD_TAGS_MAX}.`}
      confirmLabel="Done"
      isPending={save.isPending}
      confirmDisabled={!ready}
      error={error}
      onConfirm={done}
    >
      {carried.isError ? (
        <p className="text-sm text-muted-foreground">
          The dashboard's tags could not be read.
        </p>
      ) : !ready ? (
        <Spinner className="mx-auto size-4" />
      ) : (
        <div className="flex max-h-64 flex-col gap-0.5 overflow-y-auto">
          {choices.map((tag) => (
            <TagChoice
              key={tag}
              tag={tag}
              checked={chosen.includes(tag)}
              onToggle={() => toggle(tag)}
            />
          ))}
        </div>
      )}
      <Input
        aria-label="Add tag"
        placeholder="Add tag…"
        readOnly={save.isPending || !ready}
        value={draft}
        onChange={(event) => {
          setDraft(event.target.value);
          setError(null);
        }}
        onKeyDown={(event) => {
          if (event.key !== "Enter") return;
          event.preventDefault();
          addDraft();
        }}
      />
    </ConfirmDialog>
  );
}

function TagChoice({
  tag,
  checked,
  onToggle,
}: {
  tag: string;
  checked: boolean;
  onToggle: () => void;
}) {
  const id = useId();

  return (
    <label
      htmlFor={id}
      className="flex cursor-pointer items-center gap-2 rounded-sm px-1 py-1.5 text-sm hover:bg-muted"
    >
      <Checkbox id={id} checked={checked} onCheckedChange={onToggle} />
      <span className="min-w-0 truncate">{tag}</span>
    </label>
  );
}
