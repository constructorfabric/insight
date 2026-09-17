import { createFileRoute, useRouterState } from "@tanstack/react-router";

import { EditorPage } from "@/components/custom/editor/editor-page";
import { kindFromPath, nameFromPath } from "@/lib/custom/editor/route";

export const Route = createFileRoute("/portal/custom/edit/$kind/$name")({
  component: EditDefinition,
});

function EditDefinition() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const kind = kindFromPath(pathname, "edit");
  const name = nameFromPath(pathname);

  if (kind === undefined || name === undefined) return null;

  return <EditorPage kind={kind} name={name} />;
}
