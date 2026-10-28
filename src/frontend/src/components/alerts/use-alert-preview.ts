import { useQuery } from "@tanstack/react-query";

import type { AlertPreviewRequest } from "@/api/alerts-types";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { alertPreviewQuery } from "@/queries/alerts";

/** Long enough that picking a metric and a window is one run, not three. */
const RUN_DEBOUNCE_MS = 600;

export function useAlertPreview(request: AlertPreviewRequest, ready: boolean) {
  const asking = JSON.stringify(request);
  const asked = useDebouncedValue(asking, RUN_DEBOUNCE_MS);
  const settled = asked === asking;
  const query = useQuery({
    ...alertPreviewQuery(JSON.parse(asked) as AlertPreviewRequest),
    enabled: ready && settled,
  });

  return { ready, settled, query };
}

export type AlertPreviewState = ReturnType<typeof useAlertPreview>;
