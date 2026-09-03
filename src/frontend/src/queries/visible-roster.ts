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

const ROSTER_PAGE_SIZE = 500;

/** Grants and roster changes land on a seed cadence, not per interaction. */
const ROSTER_STALE_TIME = 60 * 1000;

export interface VisibleRoster {
  /** Every person the caller may see, the viewer included. */
  roster: PeopleListItem[];
  isPending: boolean;
  isError: boolean;
  retry: () => void;
}

async function collectRoster(signal: AbortSignal): Promise<PeopleListItem[]> {
  const roster: PeopleListItem[] = [];
  const cursors = new Set<string>();
  let cursor: string | undefined;

  while (true) {
    const answered = await listPeople(
      {
        cursor,
        limit: ROSTER_PAGE_SIZE,
      },
      signal,
    );
    roster.push(...answered.items);

    const nextCursor = answered.next_cursor ?? undefined;
    if (!nextCursor) return roster;
    if (cursors.has(nextCursor)) {
      throw new Error("People listing returned a repeated cursor");
    }

    cursors.add(nextCursor);
    cursor = nextCursor;
  }
}

/**
 * The caller's whole visible roster.
 */
export function useVisibleRoster(enabled: boolean): VisibleRoster {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  const query = useQuery({
    queryKey: ["identity", "visible-roster", sessionScope],
    queryFn: ({ signal }) => collectRoster(signal),
    staleTime: ROSTER_STALE_TIME,
    enabled: enabled && sessionScope != null,
  });

  return {
    roster: query.data ?? [],
    isPending: query.isPending,
    isError: query.isError,
    retry: () => void query.refetch(),
  };
}
