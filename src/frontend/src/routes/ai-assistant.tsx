import { createFileRoute } from "@tanstack/react-router";

import { AiAssistantScreen } from "@/screens/ai-assistant";

export const Route = createFileRoute("/ai-assistant")({
  component: AiAssistantScreen,
});
