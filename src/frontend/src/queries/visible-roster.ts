/**
 * The canonical people roster the caller is authorized to see.
 */
import { useQuery } from "@tanstack/react-query";

import {
  listPeople,
  type PeopleListItem,
} from "@/api/identity-client";
import { useAuth } from "@/auth/use-auth";
import { sessionAuthorizationScope } from "@/auth/session-scope";

/** Rows per request — the endpoint's own default, and its cap is 500. */
const ROSTER_PAGE_SIZE = 500;

/**
 * How many pages the walk will follow.
 *
 * The whole roster is collected up front because every zone wants a head-count
 * and a member list at once, and an organisation with no reporting lines is
 * small by construction. That does not scale: past this ceiling the roster is
 * cut and `truncated` says so, rather than the browser walking a cursor for a
 * tenant this design never suited. Revisit with a paged member list — and note
 * `metric-results` refuses more than 1000 ids in one request either way.
 */
export const MAX_ROSTER_PAGES = 4;

/** Grants and roster changes land on a seed cadence, not per interaction. */
const ROSTER_STALE_TIME = 60 * 1000;

export interface VisibleRoster {
  /** Every person the caller may see, the viewer included. */
  roster: PeopleListItem[];
  /** The walk hit {@link MAX_ROSTER_PAGES} — the roster is a prefix. */
  truncated: boolean;
  isPending: boolean;
  isError: boolean;
  retry: () => void;
}

async function collectRoster(): Promise<{
  roster: PeopleListItem[];
  truncated: boolean;
}> {
  const roster: PeopleListItem[] = [];
  let cursor: string | undefined;

  for (let page = 0; page < MAX_ROSTER_PAGES; page += 1) {
    const answered = await listPeople({
      cursor,
      limit: ROSTER_PAGE_SIZE,
    });
    roster.push(...answered.items);
    cursor = answered.next_cursor ?? undefined;
    if (!cursor) return { roster, truncated: false };
  }
  return { roster, truncated: true };
}

/**
 * The caller's whole visible roster.
 */
export function useVisibleRoster(enabled: boolean): VisibleRoster {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  const query = useQuery({
    queryKey: ["identity", "visible-roster", sessionScope],
    queryFn: collectRoster,
    staleTime: ROSTER_STALE_TIME,
    enabled: enabled && sessionScope != null,
  });

  return {
    roster: query.data?.roster ?? [],
    truncated: query.data?.truncated ?? false,
    isPending: query.isPending,
    isError: query.isError,
    retry: () => void query.refetch(),
  };
}
