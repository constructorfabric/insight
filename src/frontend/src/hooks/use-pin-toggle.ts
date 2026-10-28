import { useQuery } from "@tanstack/react-query";

import { refusal } from "@/components/custom/refusal";
import { toast } from "@/components/ui/sonner";
import { pinsQuery, useSetPinned } from "@/queries/custom";

export function usePinToggle(id: string, enabled: boolean) {
  const pins = useQuery({ ...pinsQuery(), enabled });
  const setPinned = useSetPinned();

  const pinned = pins.data?.some((pin) => pin.id === id) ?? false;

  function toggle() {
    const failed = pinned
      ? "The dashboard could not be unpinned."
      : "The dashboard could not be pinned.";

    setPinned.mutate(
      { id, pinned: !pinned },
      { onError: (error) => toast.error(refusal(error, failed)) }
    );
  }

  return { pinned, ready: !pins.isPending, toggle };
}
