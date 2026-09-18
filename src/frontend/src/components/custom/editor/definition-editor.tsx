import { useQueries } from "@tanstack/react-query";
import { useState } from "react";

import type { EditableKind } from "@/api/custom-client";
import { FieldsView } from "@/components/custom/editor/fields-view";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import type { Path } from "@/lib/custom/editor/describe";
import {
  change,
  hold,
  retype,
  sendable,
  type Held,
} from "@/lib/custom/editor/document";
import { DESCRIPTIONS } from "@/lib/custom/editor/kinds";
import { place } from "@/lib/custom/editor/violations";
import { catalogueNamesQuery, useStoreDefinition } from "@/queries/custom";
import { TEXT_BODY, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

const KINDS: EditableKind[] = ["datasets", "metrics", "widgets", "dashboards"];

export function DefinitionEditor({
  kind,
  name,
  document,
  onStored,
}: {
  kind: EditableKind;
  /** The definition being edited. A new one has no name until it is given. */
  name?: string;
  document?: Record<string, unknown>;
  onStored: (name: string) => void;
}) {
  const description = DESCRIPTIONS[kind];
  const [called, setCalled] = useState(name ?? "");
  const [held, setHeld] = useState<Held>(() => hold(document));
  const [view, setView] = useState<"fields" | "text">("fields");
  const store = useStoreDefinition();

  const catalogues = useQueries({
    queries: KINDS.map((each) => catalogueNamesQuery(each)),
  });
  const stored = new Map(
    KINDS.map((each, index) => [each, catalogues[index]?.data ?? []])
  );
  const names = (of: EditableKind) => stored.get(of) ?? [];

  const placed = place(
    store.error,
    description.fields,
    held.document,
    "Couldn't store it."
  );
  const given = called.trim();

  const submit = () => {
    if (given === "" || !sendable(held)) return;
    store.mutate(
      { kind, name: given, body: held.document },
      { onSuccess: () => onStored(given) }
    );
  };

  return (
    <form
      className="flex flex-col gap-5"
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
      // SAFETY: storing claims a name and, for a dataset, builds what holds its
      // records. Enter in a field is not that decision; only Save is.
      onKeyDown={(event) => {
        const inField = event.target instanceof HTMLInputElement;
        if (event.key === "Enter" && inField) event.preventDefault();
      }}
    >
      <div className="flex flex-col gap-1">
        <label
          htmlFor="definition-name"
          className={cn(TEXT_LABEL, "font-medium")}
        >
          Name
        </label>
        <Input
          id="definition-name"
          value={called}
          readOnly={name !== undefined}
          className="h-9 w-72 font-mono"
          onChange={(event) => setCalled(event.target.value)}
        />
        {name === undefined ? (
          <p className={cn(TEXT_LABEL, "text-muted-foreground")}>
            What everything else will call this {description.noun}.
          </p>
        ) : null}
      </div>

      {placed.loose.map((said) => (
        <p
          key={said}
          role="alert"
          className={cn(TEXT_BODY, "text-destructive")}
        >
          {said}
        </p>
      ))}

      <div className="flex items-center gap-2">
        {(["fields", "text"] as const).map((each) => (
          <Button
            key={each}
            type="button"
            variant={view === each ? "secondary" : "ghost"}
            size="sm"
            aria-pressed={view === each}
            onClick={() => setView(each)}
          >
            {each === "fields" ? "Fields" : "Text"}
          </Button>
        ))}
      </div>

      {view === "fields" ? (
        <FieldsView
          fields={description.fields}
          at={[]}
          editing={{
            document: held.document,
            said: placed.at,
            names,
            onChange: (path: Path, value: unknown) =>
              setHeld((was) => change(was, path, value)),
          }}
        />
      ) : (
        <div className="flex flex-col gap-1">
          <label htmlFor="definition-text" className="sr-only">
            {description.noun} as text
          </label>
          <Textarea
            id="definition-text"
            value={held.text}
            spellCheck={false}
            className="min-h-96 font-mono"
            onChange={(event) =>
              setHeld((was) => retype(was, event.target.value))
            }
          />
          {held.unparsed ? (
            <p role="alert" className={cn(TEXT_LABEL, "text-destructive")}>
              {held.unparsed}
            </p>
          ) : null}
          {placed.at.size === 0 ? null : (
            <ul
              className={cn(
                TEXT_LABEL,
                "flex flex-col gap-0.5 text-destructive"
              )}
            >
              {[...placed.at].map(([path, said]) => (
                <li key={path} role="alert">
                  <span className="font-mono">{path}</span>: {said}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      <div className="flex items-center gap-2">
        <Button
          type="submit"
          disabled={given === "" || !sendable(held) || store.isPending}
        >
          {store.isPending ? <Spinner className="size-3" /> : null}
          Save
        </Button>
        {held.unparsed ? (
          <span className={cn(TEXT_LABEL, "text-muted-foreground")}>
            The text has to parse before this can be sent.
          </span>
        ) : null}
      </div>
    </form>
  );
}
