import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";

import { DEFINITION_NAME_MAX, type EditableKind } from "@/api/custom-client";
import { TextView } from "@/components/custom/editor/text-view";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import {
  hold,
  retype,
  sendable,
  type Held,
} from "@/lib/custom/editor/document";
import { NOUNS } from "@/lib/custom/editor/kinds";
import { refusals } from "@/lib/custom/editor/violations";
import { catalogueQuery, useStoreDefinition } from "@/queries/custom";
import { TEXT_BODY, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export function DefinitionEditor({
  kind,
  id,
  name,
  document,
  onStored,
}: {
  kind: EditableKind;
  id?: string;
  name?: string;
  document?: Record<string, unknown>;
  onStored: (id: string) => void;
}) {
  const noun = NOUNS[kind];
  const [called, setCalled] = useState(name ?? "");
  const [held, setHeld] = useState<Held>(() => hold(document));
  const store = useStoreDefinition();

  const datasetCatalogue = useQuery(catalogueQuery("datasets"));
  const datasets = () => datasetCatalogue.data?.map((item) => item.id) ?? [];

  const refused = refusals(store.error, "Couldn't store it.");
  const given = called.trim();
  const isDataset = kind === "datasets";
  const creating = id === undefined;
  // SAFETY: writing a dataset replaces the one under its name, so a new one
  // may not take a name the catalogue lists; the catalogue may lag the service.
  const taken = isDataset && creating && datasets().includes(given);

  const submit = () => {
    if (given === "" || taken || !sendable(held)) return;
    store.mutate(
      { kind, id, name: given, body: held.document },
      { onSuccess: (storedId) => onStored(storedId) }
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
      {isDataset && !creating ? (
        <div className="flex flex-col gap-1">
          <span className={cn(TEXT_LABEL, "font-medium")}>Name</span>
          <span className={cn(TEXT_BODY, "font-mono")}>{name}</span>
          <p className={cn(TEXT_LABEL, "text-muted-foreground")}>
            A dataset keeps its name: records are sent to it by this name.
          </p>
        </div>
      ) : (
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
            maxLength={isDataset ? undefined : DEFINITION_NAME_MAX}
            aria-invalid={taken || undefined}
            aria-describedby={isDataset ? "definition-name-hint" : undefined}
            className={cn("h-9 w-full max-w-72", isDataset && "font-mono")}
            onChange={(event) => setCalled(event.target.value)}
          />
          {isDataset ? (
            <p
              id="definition-name-hint"
              className={cn(TEXT_LABEL, "text-muted-foreground")}
            >
              The identifier: what the API path, metrics and anything sending
              records call this dataset. Letters, digits, <code>_</code> and{" "}
              <code>-</code>.
            </p>
          ) : null}
          {taken ? (
            <p role="alert" className={cn(TEXT_LABEL, "text-destructive")}>
              A {noun} called <code>{given}</code> already exists.{" "}
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
      )}

      <div className="flex flex-col gap-1">
        <TextView
          id="definition-text"
          label={`${noun} as text`}
          text={held.text}
          unparsed={held.unparsed}
          onChange={(text) => setHeld((was) => retype(was, text))}
        />
        {refused.length === 0 ? null : (
          <ul
            className={cn(TEXT_LABEL, "flex flex-col gap-0.5 text-destructive")}
          >
            {refused.map((said, index) => (
              <li key={`${index}:${said}`} role="alert">
                {said}
              </li>
            ))}
          </ul>
        )}
      </div>

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
