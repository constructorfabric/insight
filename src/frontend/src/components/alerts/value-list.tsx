import { useState, type KeyboardEvent } from "react";

import {
  Combobox,
  ComboboxChip,
  ComboboxChips,
  ComboboxChipsInput,
} from "@/components/ui/combobox";
import { withListValues } from "@/lib/alerts/condition";
import { saidId } from "@/lib/custom/editor/aria";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

import { described } from "./condition-aria";

/** Keys that end an entry and add it as a value. */
const ADDING_KEYS = new Set(["Enter", ","]);

/**
 * Values typed one at a time and held as chips. There is nothing to suggest
 * from, so the list never opens.
 */
export function ValueListInput({
  id,
  value,
  describedBy,
  onChange,
}: {
  id: string;
  value: unknown;
  describedBy: readonly string[];
  onChange: (values: unknown[]) => void;
}) {
  const values = Array.isArray(value) ? (value as unknown[]) : [];
  const [typed, setTyped] = useState("");
  const [refused, setRefused] = useState<string>();
  const refusedId = saidId(`${id}-entry`);

  function add(entries: readonly string[], rest: string) {
    const added = withListValues(values, entries);
    setRefused(added.refused);
    setTyped([...added.unadded, rest].filter((part) => part !== "").join(","));
    if (added.values.length !== values.length) onChange(added.values);
  }

  function type(event: KeyboardEvent<HTMLInputElement>) {
    if (!ADDING_KEYS.has(event.key)) return;

    event.preventDefault();
    (event as { preventBaseUIHandler?: () => void }).preventBaseUIHandler?.();
    add([typed], "");
  }

  return (
    <div className="flex flex-col gap-1">
      <Combobox<unknown, true>
        multiple
        open={false}
        value={values}
        inputValue={typed}
        onValueChange={(next) => {
          setRefused(undefined);
          onChange(next);
        }}
        onInputValueChange={(text) => {
          if (!text.includes(",")) {
            setRefused(undefined);
            setTyped(text);
            return;
          }
          const entries = text.split(",");
          const rest = entries.pop() ?? "";
          add(entries, rest);
        }}
      >
        <ComboboxChips className="w-64">
          {values.map((item, index) => (
            <ComboboxChip key={index} removeLabel={`Remove ${String(item)}`}>
              {String(item)}
            </ComboboxChip>
          ))}
          <ComboboxChipsInput
            id={id}
            aria-label="Values"
            {...described(refused ? [...describedBy, refusedId] : describedBy)}
            onKeyDown={type}
          />
        </ComboboxChips>
      </Combobox>
      {refused ? (
        <p
          id={refusedId}
          role="alert"
          className={cn(TEXT_LABEL, "text-destructive")}
        >
          {refused}
        </p>
      ) : null}
    </div>
  );
}
