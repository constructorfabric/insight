import { createFileRoute, useRouterState } from "@tanstack/react-router";

import { EditorPage } from "@/components/custom/editor/editor-page";
import { kindFromPath } from "@/lib/custom/editor/route";

export const Route = createFileRoute("/portal/custom/new/$kind")({
  component: NewDefinition,
});

function NewDefinition() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const kind = kindFromPath(pathname, "new");

  return kind === undefined ? null : <EditorPage kind={kind} />;
}
