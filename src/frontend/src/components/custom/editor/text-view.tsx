import { useRef, type KeyboardEvent, type UIEvent } from "react";

import { brokenLine } from "@/lib/custom/editor/document";
import {
  indent,
  newline,
  outdent,
  type Typed,
} from "@/lib/custom/editor/typing";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/**
 * The document as text, with a line for every line.
 *
 * Lines do not wrap: a wrapped line would take two rows against one number,
 * and the parser's "line 18" would point at the wrong place.
 */
export function TextView({
  id,
  label,
  text,
  unparsed,
  onChange,
}: {
  id: string;
  label: string;
  text: string;
  /** Why the text cannot be believed, as the parser said it. */
  unparsed?: string;
  onChange: (text: string) => void;
}) {
  const gutter = useRef<HTMLDivElement>(null);
  const lines = text.split("\n").length;
  const broken = brokenLine(unparsed);

  const follow = (event: UIEvent<HTMLTextAreaElement>) => {
    if (gutter.current)
      gutter.current.scrollTop = event.currentTarget.scrollTop;
  };

  // Tab indents rather than leaving the field; Escape is the way out for a
  // keyboard, since the field has taken Tab.
  const type = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    const field = event.currentTarget;
    const typed = keystroke(event, field);
    if (typed === undefined) return;

    event.preventDefault();
    onChange(typed.text);
    requestAnimationFrame(() =>
      field.setSelectionRange(typed.start, typed.end)
    );
  };

  return (
    <div className="flex flex-col gap-1">
      <label htmlFor={id} className="sr-only">
        {label}
      </label>

      <div
        className={cn(
          "flex min-h-96 overflow-hidden rounded-md border bg-transparent font-mono text-sm",
          unparsed ? "border-destructive" : "border-input"
        )}
      >
        <div
          ref={gutter}
          aria-hidden="true"
          className="overflow-hidden border-e border-input bg-muted/40 py-2 text-end text-muted-foreground select-none"
        >
          {Array.from({ length: lines }, (_, index) => (
            <div
              key={index}
              className={cn(
                "px-2 leading-6",
                index + 1 === broken && "bg-destructive/20 text-destructive"
              )}
            >
              {index + 1}
            </div>
          ))}
        </div>
        <textarea
          id={id}
          value={text}
          spellCheck={false}
          wrap="off"
          aria-invalid={unparsed ? true : undefined}
          aria-describedby={unparsed ? `${id}-unparsed` : undefined}
          className="min-h-96 grow resize-y bg-transparent px-3 py-2 leading-6 outline-none"
          onScroll={follow}
          onKeyDown={type}
          onChange={(event) => onChange(event.target.value)}
        />
      </div>

      {unparsed ? (
        <p
          id={`${id}-unparsed`}
          role="alert"
          className={cn(TEXT_LABEL, "text-destructive")}
        >
          {unparsed}
        </p>
      ) : null}
    </div>
  );
}

function keystroke(
  event: KeyboardEvent<HTMLTextAreaElement>,
  field: HTMLTextAreaElement
): Typed | undefined {
  const { value, selectionStart, selectionEnd } = field;

  if (event.key === "Escape") {
    field.blur();
    return undefined;
  }
  if (event.key === "Tab") {
    return event.shiftKey
      ? outdent(value, selectionStart, selectionEnd)
      : indent(value, selectionStart, selectionEnd);
  }
  if (event.key === "Enter" && !event.metaKey && !event.ctrlKey) {
    return newline(value, selectionStart, selectionEnd);
  }

  return undefined;
}
