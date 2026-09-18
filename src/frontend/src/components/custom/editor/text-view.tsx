import { useRef, type UIEvent } from "react";

import { brokenLine } from "@/lib/custom/editor/document";
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
