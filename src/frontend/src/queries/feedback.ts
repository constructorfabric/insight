import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseQueryResult,
} from "@tanstack/react-query";

import {
  getFeedback,
  submitFeedback,
  type FeedbackList,
  type FeedbackRange,
  type FeedbackSubmission,
} from "@/api/feedback-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";

const LIST_KEY = ["feedback", "list"] as const;

export function useSubmitFeedback() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (body: FeedbackSubmission) => submitFeedback(body),
    onSuccess: () => client.invalidateQueries({ queryKey: LIST_KEY }),
  });
}

export function useFeedbackList(
  range: FeedbackRange,
): UseQueryResult<FeedbackList> {
  const { session } = useAuth();
  return useQuery({
    queryKey: [
      ...LIST_KEY,
      sessionAuthorizationScope(session),
      range.since,
      range.until,
    ],
    queryFn: () => getFeedback(range),
    staleTime: 0,
    refetchOnMount: "always",
  });
}
