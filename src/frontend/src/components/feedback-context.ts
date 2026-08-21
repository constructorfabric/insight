import { createContext, useContext } from "react";

export interface FeedbackDialogContextValue {
  openFeedback: () => void;
}

export const FeedbackDialogContext = createContext<
  FeedbackDialogContextValue | undefined
>(undefined);

export function useFeedbackDialog(): FeedbackDialogContextValue {
  const context = useContext(FeedbackDialogContext);
  if (!context) {
    throw new Error(
      "useFeedbackDialog must be used within FeedbackDialogProvider",
    );
  }
  return context;
}
