/**
 * The portal renders the sidebar footer inside a settings POPOVER, so a dialog
 * owned by the button that opens it unmounts the moment the dialog takes focus.
 * It is mounted here instead.
 */
import { useMemo, useState, type ReactNode } from "react";

import { FeedbackDialogContext } from "@/components/feedback-context";
import { FeedbackDialog } from "@/components/feedback-dialog";

export function FeedbackDialogProvider({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const value = useMemo(() => ({ openFeedback: () => setOpen(true) }), []);

  return (
    <FeedbackDialogContext.Provider value={value}>
      {children}
      <FeedbackDialog open={open} onOpenChange={setOpen} />
    </FeedbackDialogContext.Provider>
  );
}
