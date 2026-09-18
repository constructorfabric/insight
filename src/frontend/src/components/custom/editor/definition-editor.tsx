import { useQueries, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";

import type { EditableKind } from "@/api/custom-client";
import { FieldsView } from "@/components/custom/editor/fields-view";
import { TextView } from "@/components/custom/editor/text-view";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
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
import {
  catalogueNamesQuery,
  datasetQuery,
  useStoreDefinition,
} from "@/queries/custom";
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

  const reads = held.document.dataset;
  const dataset = useQuery({
    ...datasetQuery(typeof reads === "string" ? reads : ""),
    enabled: typeof reads === "string" && reads !== "",
  });
  const declared = () => dataset.data?.declaration.fields ?? [];

  const placed = place(
    store.error,
    description.fields,
    held.document,
    "Couldn't store it."
  );
  const given = called.trim();
  // SAFETY: the service replaces on write. A new definition given a name the
  // catalogue already holds would write over that one, so the form refuses
  // what it can see is taken; the catalogue may run a page behind, so this is
  // a guard against a slip, not the service's own check.
  const taken = name === undefined && names(kind).includes(given);

  const submit = () => {
    if (given === "" || taken || !sendable(held)) return;
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
      {name === undefined ? (
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
            aria-invalid={taken || undefined}
            aria-describedby="definition-name-hint"
            className="h-9 w-72 font-mono"
            onChange={(event) => setCalled(event.target.value)}
          />
          <p
            id="definition-name-hint"
            className={cn(TEXT_LABEL, "text-muted-foreground")}
          >
            The identifier: what the API path, other definitions and anything
            sending records call this {description.noun}. Letters, digits,{" "}
            <code>_</code> and <code>-</code>.
          </p>
          {taken ? (
            <p role="alert" className={cn(TEXT_LABEL, "text-destructive")}>
              A {description.noun} called <code>{given}</code> already exists.{" "}
              <Link
                to="/portal/custom/edit/$kind/$name"
                params={{ kind, name: given }}
                className="underline decoration-dotted underline-offset-4"
              >
                Open it
              </Link>{" "}
              instead of writing over it.
            </p>
          ) : null}
        </div>
      ) : (
        <div className="flex flex-col gap-1">
          <span className={cn(TEXT_LABEL, "font-medium")}>Name</span>
          <span className={cn(TEXT_BODY, "font-mono")}>{name}</span>
          <p className={cn(TEXT_LABEL, "text-muted-foreground")}>
            {kind === "datasets"
              ? "A dataset keeps its name: records are sent to it by this name."
              : "Renamed under Rename, below, so everything that names it follows."}
          </p>
        </div>
      )}

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
            {each === "fields" ? "Editor" : "JSON"}
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
            declared,
            onChange: (path: Path, value: unknown) =>
              setHeld((was) => change(was, path, value)),
          }}
        />
      ) : (
        <div className="flex flex-col gap-1">
          <TextView
            id="definition-text"
            label={`${description.noun} as text`}
            text={held.text}
            unparsed={held.unparsed}
            onChange={(text) => setHeld((was) => retype(was, text))}
          />
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
          disabled={given === "" || taken || !sendable(held) || store.isPending}
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
