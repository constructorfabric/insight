import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import type { SavedQuerySummary } from "@/api/saved-queries-client";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { SidebarTrigger } from "@/components/ui/sidebar";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { QueryDetail } from "@/components/widgets/query-console/query-detail";
import {
  QueryEditorDialog,
  type QueryDraft,
} from "@/components/widgets/query-console/query-editor-dialog";
import {
  useCreateSavedQuery,
  useDeleteSavedQuery,
  useSavedQueries,
  useSavedQuery,
  useUpdateSavedQuery,
} from "@/queries/saved-queries";
import { Plus, Terminal, TriangleAlert } from "lucide-react";

type DialogState =
  | { kind: "closed" }
  | { kind: "create" }
  | { kind: "edit"; id: string };

export function QueryConsoleScreen() {
  const { t } = useTranslation();
  const { data: queries, isPending, isError } = useSavedQueries();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [dialog, setDialog] = useState<DialogState>({ kind: "closed" });

  const createQuery = useCreateSavedQuery();
  const deleteQuery = useDeleteSavedQuery();

  const closeDialog = () => setDialog({ kind: "closed" });

  const handleDelete = (id: string) => {
    deleteQuery.mutate(id, {
      onSuccess: () => {
        if (selectedId === id) setSelectedId(null);
      },
    });
  };

  return (
    <>
      <header className="sticky top-0 z-20 flex flex-wrap items-center gap-3 border-b bg-background/95 px-4 py-3 backdrop-blur-sm">
        <SidebarTrigger />
        <h1 className="text-xl font-semibold tracking-tight">
          {t("query_console.title")}
        </h1>
        <Button
          className="ms-auto"
          size="sm"
          onClick={() => setDialog({ kind: "create" })}
        >
          <Plus />
          {t("query_console.new_query")}
        </Button>
      </header>

      <main className="flex flex-1 flex-col p-4 md:p-6">
        {isPending ? <CenteredSpinner className="min-h-[70vh]" /> : null}

        {isError ? (
          <Alert variant="destructive">
            <TriangleAlert />
            <AlertTitle>{t("query_console.list_error")}</AlertTitle>
            <AlertDescription>
              {t("query_console.list_error_description")}
            </AlertDescription>
          </Alert>
        ) : null}

        {deleteQuery.isError ? (
          <Alert variant="destructive" className="mb-4">
            <TriangleAlert />
            <AlertTitle>{t("query_console.delete_error")}</AlertTitle>
          </Alert>
        ) : null}

        {!isPending && !isError ? (
          <div className="mx-auto grid w-full max-w-6xl flex-1 grid-cols-1 gap-6 md:grid-cols-[16rem_1fr]">
            <QueryList
              queries={queries ?? []}
              selectedId={selectedId}
              onSelect={setSelectedId}
            />
            <section className="min-w-0">
              {selectedId ? (
                <QueryDetail
                  key={selectedId}
                  id={selectedId}
                  onEdit={(id) => setDialog({ kind: "edit", id })}
                  onDelete={handleDelete}
                />
              ) : (
                <Empty className="min-h-[50vh]">
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      <Terminal />
                    </EmptyMedia>
                    <EmptyTitle>{t("query_console.no_selection")}</EmptyTitle>
                    <EmptyDescription>
                      {t("query_console.no_selection_description")}
                    </EmptyDescription>
                  </EmptyHeader>
                </Empty>
              )}
            </section>
          </div>
        ) : null}
      </main>

      {dialog.kind === "create" ? (
        <QueryEditorDialog
          open
          onOpenChange={(open) => (open ? null : closeDialog())}
          mode="create"
          isPending={createQuery.isPending}
          error={createQuery.error}
          onSubmit={(draft) =>
            createQuery.mutate(toRequest(draft), {
              onSuccess: (created) => {
                setSelectedId(created.id);
                closeDialog();
              },
            })
          }
        />
      ) : null}

      {dialog.kind === "edit" ? (
        <EditQueryDialog id={dialog.id} onClose={closeDialog} />
      ) : null}
    </>
  );
}

function QueryList({
  queries,
  selectedId,
  onSelect,
}: {
  queries: SavedQuerySummary[];
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  const { t } = useTranslation();

  if (queries.length === 0) {
    return (
      <aside className="text-sm text-muted-foreground">
        {t("query_console.empty_list")}
      </aside>
    );
  }

  return (
    <aside className="flex flex-col gap-1">
      {queries.map((query) => (
        <button
          key={query.id}
          type="button"
          onClick={() => onSelect(query.id)}
          className={`rounded-md border px-3 py-2 text-start text-sm transition-colors ${
            query.id === selectedId
              ? "border-ring bg-muted"
              : "border-transparent hover:bg-muted/60"
          }`}
        >
          <div className="truncate font-medium">{query.name}</div>
          {query.description ? (
            <div className="truncate text-xs text-muted-foreground">
              {query.description}
            </div>
          ) : null}
        </button>
      ))}
    </aside>
  );
}

/**
 * Edit fetches the full query (list summaries carry no `sql`) so the form
 * prefills; kept a child so the fetch hook mounts only while editing.
 */
function EditQueryDialog({
  id,
  onClose,
}: {
  id: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const { data: query, isError } = useSavedQuery(id);
  const updateQuery = useUpdateSavedQuery(id);

  // The form must mount once with stable prefill values, so wait for the full
  // query (list summaries carry no `sql`). The detail pane has usually cached
  // it, but a slow fetch or an error still gets visible feedback rather than a
  // silent no-op dialog.
  if (isError) {
    return (
      <StatusDialog onClose={onClose}>
        <Alert variant="destructive">
          <TriangleAlert />
          <AlertTitle>{t("query_console.detail.load_error")}</AlertTitle>
        </Alert>
      </StatusDialog>
    );
  }
  if (!query) {
    return (
      <StatusDialog onClose={onClose}>
        <CenteredSpinner className="min-h-40" />
      </StatusDialog>
    );
  }

  return (
    <QueryEditorDialog
      open
      onOpenChange={(open) => (open ? null : onClose())}
      mode="edit"
      initial={{
        name: query.name,
        description: query.description ?? "",
        sql: query.sql,
      }}
      isPending={updateQuery.isPending}
      error={updateQuery.error}
      onSubmit={(draft) =>
        updateQuery.mutate(toRequest(draft), { onSuccess: onClose })
      }
    />
  );
}

/** Minimal modal shell for the edit dialog's loading and error states. */
function StatusDialog({
  onClose,
  children,
}: {
  onClose: () => void;
  children: ReactNode;
}) {
  return (
    <Dialog open onOpenChange={(open) => (open ? null : onClose())}>
      <DialogContent className="sm:max-w-md">{children}</DialogContent>
    </Dialog>
  );
}

/** Blank description clears the field (send `null`) rather than storing "". */
function toRequest(draft: QueryDraft): {
  name: string;
  description: string | null;
  sql: string;
} {
  const description = draft.description.trim();
  return {
    name: draft.name.trim(),
    description: description === "" ? null : description,
    sql: draft.sql,
  };
}
