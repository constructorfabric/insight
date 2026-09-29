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
import type { SortDirection } from "@/api/usage-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";
import { keepWithinPeriod } from "@/queries/same-period";

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
  direction: SortDirection | null = null,
): UseQueryResult<FeedbackList> {
  const { session } = useAuth();
  return useQuery({
    queryKey: [
      ...LIST_KEY,
      sessionAuthorizationScope(session),
      range.since,
      range.until,
      direction,
    ],
    queryFn: () => getFeedback(range, direction),
    staleTime: 0,
    refetchOnMount: "always",
    placeholderData: keepWithinPeriod<FeedbackList>(range),
  });
}
