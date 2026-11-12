import { useId, useLayoutEffect, useRef, useState } from "react";
import { Input } from "@gears-frontx/ui-kit";

function enteredMinute(raw: string, digits: string, replace: boolean): string {
  let next = replace ? "" : raw;
  for (const digit of digits) next = next.length === 2 ? digit : next + digit;
  return next;
}

export function MinuteField({
  value,
  onChange,
}: {
  value: string;
  onChange: (minute: string) => void;
}) {
  const id = useId();
  const [minute, setMinute] = useState(Number(value));
  const [raw, setRaw] = useState(value ? String(Number(value)) : "");
  const [focused, setFocused] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const selectAfterUpdate = useRef(false);
  const replaceNext = useRef(true);
  const valid = /^\d{1,2}$/.test(raw) && Number(raw) <= 59;

  useLayoutEffect(() => {
    if (selectAfterUpdate.current) {
      input.current?.select();
      selectAfterUpdate.current = false;
    }
    const element = input.current;
    function beforeInput(event: InputEvent) {
      if (
        event.inputType !== "insertText" ||
        !event.data ||
        !/^\d+$/.test(event.data)
      )
        return;
      event.preventDefault();
      enter(event.data);
    }
    element?.addEventListener("beforeinput", beforeInput);
    return () => element?.removeEventListener("beforeinput", beforeInput);
  });

  function enter(digits: string) {
    const next = enteredMinute(raw, digits, replaceNext.current);
    replaceNext.current = false;
    update(next);
  }

  function update(raw: string) {
    if (!/^\d{0,2}$/.test(raw)) return;
    setRaw(raw);
    const valid = raw !== "" && Number(raw) <= 59;
    if (valid) setMinute(Number(raw));
    onChange(valid ? String(Number(raw)).padStart(2, "0") : "");
  }

  return (
    <div>
      <Input
        ref={input}
        aria-label="Minute"
        type="text"
        inputMode="numeric"
        role="spinbutton"
        aria-valuemin={0}
        aria-valuemax={59}
        aria-valuenow={valid ? minute : undefined}
        aria-invalid={!valid}
        aria-describedby={!valid ? id : undefined}
        maxLength={2}
        className="h-8 w-[64px] max-w-full min-w-0 font-mono"
        value={focused || !valid ? raw : String(minute).padStart(2, "0")}
        onFocus={(event) => {
          setFocused(true);
          replaceNext.current = true;
          event.currentTarget.select();
        }}
        onPointerDown={() => {
          replaceNext.current = true;
        }}
        onClick={(event) => {
          replaceNext.current = true;
          event.currentTarget.select();
        }}
        onBlur={() => setFocused(false)}
        onChange={(event) => update(event.target.value)}
        onPaste={(event) => {
          event.preventDefault();
          replaceNext.current = true;
          update(event.clipboardData.getData("text").replace(/^:/, ""));
        }}
        onKeyDown={(event) => {
          if (
            !event.ctrlKey &&
            !event.metaKey &&
            !event.altKey &&
            /^\d$/.test(event.key)
          ) {
            event.preventDefault();
            enter(event.key);
            return;
          }
          if (
            (event.ctrlKey || event.metaKey) &&
            event.key.toLowerCase() === "a"
          )
            replaceNext.current = true;
          const steps: Record<string, number> = {
            ArrowUp: 1,
            ArrowDown: -1,
            PageUp: 10,
            PageDown: -10,
          };
          if (
            event.key in steps ||
            event.key === "Home" ||
            event.key === "End"
          ) {
            event.preventDefault();
            selectAfterUpdate.current = true;
            replaceNext.current = true;
            update(
              String(
                event.key === "Home"
                  ? 0
                  : event.key === "End"
                    ? 59
                    : (minute + steps[event.key] + 60) % 60
              )
            );
          } else if (event.key === "Backspace") {
            event.preventDefault();
            replaceNext.current = true;
            update("");
          }
        }}
      />
      {!valid && (
        <p id={id} role="alert" className="text-xs text-destructive">
          Enter a minute from 0 to 59.
        </p>
      )}
    </div>
  );
}
