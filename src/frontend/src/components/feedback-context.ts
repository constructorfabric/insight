import { createContext, useContext } from "react";

export interface FeedbackDialogContextValue {
  openFeedback: () => void;
}

export const FeedbackDialogContext = createContext<
  FeedbackDialogContextValue | undefined
>(undefined);

/** Undefined where no provider is mounted — the caller hides its trigger. */
export function useFeedbackDialog(): FeedbackDialogContextValue | undefined {
  return useContext(FeedbackDialogContext);
}
