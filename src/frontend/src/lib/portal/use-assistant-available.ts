import { usePortalShowPlanned } from "@/lib/portal/portal-store";
import { useIsAdmin } from "@/queries/identity-me";

export function useAssistantAvailable(): boolean {
  const { isAdmin } = useIsAdmin();
  const showPlanned = usePortalShowPlanned();
  return isAdmin && showPlanned;
}
