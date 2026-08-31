import { createFileRoute } from "@tanstack/react-router";

import { GearItemsScreen } from "@/screens/gear-items";

export const Route = createFileRoute("/gears/items")({
  component: GearItemsScreen,
});
