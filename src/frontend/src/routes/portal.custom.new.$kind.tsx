import { createFileRoute, useRouterState } from "@tanstack/react-router";

import { EditorPage } from "@/components/custom/editor/editor-page";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { kindFromPath } from "@/lib/custom/editor/route";

export const Route = createFileRoute("/portal/custom/new/$kind")({
  component: NewDefinition,
});

function NewDefinition() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const kind = kindFromPath(pathname, "new");

  if (kind === undefined) return <NoSuchKind />;

  return <EditorPage kind={kind} />;
}

function NoSuchKind() {
  return (
    <div className="p-4 md:p-6">
      <ComingSoon
        variant="card"
        state="empty"
        label="There is no such kind of definition."
      />
    </div>
  );
}
