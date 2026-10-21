import { createFileRoute, useRouterState } from "@tanstack/react-router";

import { EditorPage } from "@/components/custom/editor/editor-page";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { idFromPath, kindFromPath } from "@/lib/custom/editor/route";

export const Route = createFileRoute("/portal/custom/edit/$kind/$name")({
  component: EditDefinition,
});

function EditDefinition() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const kind = kindFromPath(pathname, "edit");
  const id = idFromPath(pathname);

  if (kind === undefined || id === undefined) return <NoSuchKind />;

  return <EditorPage kind={kind} id={id} />;
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
