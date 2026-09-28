import { useQuery } from "@tanstack/react-query";

import type { TagSummary } from "@/api/custom-client";
import { sameTagName } from "@/lib/custom/tag-names";
import {
  usePortalSearch,
  useSetPortalSearch,
} from "@/lib/portal/portal-search";
import { tagsQuery } from "@/queries/custom";

export interface TagPicks {
  tags: TagSummary[] | undefined;
  picked: string[];
  pending: boolean;
  pick: (next: string[]) => void;
}

export function useTagPicks(): TagPicks {
  const { tag: asked } = usePortalSearch();
  const list = useQuery(tagsQuery());
  const setSearch = useSetPortalSearch();

  const tags = list.data?.tags;
  const pick = (next: string[]) =>
    setSearch({ tag: next.length > 0 ? next : undefined });

  if (!asked) return { tags, picked: [], pending: false, pick };
  if (list.isError) return { tags, picked: asked, pending: false, pick };
  if (!tags) return { tags, picked: [], pending: true, pick };

  const picked = tags
    .map((tag) => tag.name)
    .filter((name) => asked.some((one) => sameTagName(one, name)));

  return { tags, picked, pending: false, pick };
}
