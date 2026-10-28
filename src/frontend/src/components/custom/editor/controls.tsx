import type { ReactNode } from "react";

import { hintId, saidId } from "@/lib/custom/editor/aria";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export function Row({
  id,
  label,
  property,
  hint,
  required,
  said,
  children,
}: {
  id: string;
  label: string;
  /** The property's name in the document, as the text view spells it. */
  property?: string;
  hint?: string;
  required?: boolean;
  said?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1">
      <span className={cn(TEXT_LABEL, "flex items-baseline gap-1 font-medium")}>
        <label htmlFor={id}>{label}</label>
        {property ? (
          <span
            aria-hidden="true"
            className="font-mono font-normal text-muted-foreground"
          >
            ({property})
          </span>
        ) : null}
        {required ? <span aria-hidden="true">*</span> : null}
      </span>

      {children}

      {hint ? (
        <p id={hintId(id)} className={cn(TEXT_LABEL, "text-muted-foreground")}>
          {hint}
        </p>
      ) : null}
      {said ? (
        <p
          id={saidId(id)}
          role="alert"
          className={cn(TEXT_LABEL, "text-destructive")}
        >
          {said}
        </p>
      ) : null}
    </div>
  );
}
