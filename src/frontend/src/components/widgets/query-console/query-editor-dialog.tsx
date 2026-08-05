import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { apiErrorReason } from "@/lib/query-console/api-error";
import { TriangleAlert } from "lucide-react";

export interface QueryDraft {
  name: string;
  description: string;
  sql: string;
}

export interface QueryEditorDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  mode: "create" | "edit";
  /** Prefill for edit; ignored for create. */
  initial?: QueryDraft;
  onSubmit: (draft: QueryDraft) => void;
  isPending: boolean;
  error: unknown;
}

const EMPTY_DRAFT: QueryDraft = { name: "", description: "", sql: "" };

export function QueryEditorDialog({
  open,
  onOpenChange,
  mode,
  initial,
  onSubmit,
  isPending,
  error,
}: QueryEditorDialogProps) {
  const { t } = useTranslation();
  // The parent mounts this dialog only while open, so the initializer runs
  // fresh on each open: create starts blank, edit prefills from `initial`.
  const [draft, setDraft] = useState<QueryDraft>(initial ?? EMPTY_DRAFT);

  const canSubmit =
    draft.name.trim() !== "" && draft.sql.trim() !== "" && !isPending;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {mode === "create"
              ? t("query_console.editor.create_title")
              : t("query_console.editor.edit_title")}
          </DialogTitle>
        </DialogHeader>

        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            if (canSubmit) onSubmit(draft);
          }}
        >
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="query-name">
              {t("query_console.editor.name_label")}
            </Label>
            <Input
              id="query-name"
              value={draft.name}
              onChange={(event) =>
                setDraft((d) => ({ ...d, name: event.target.value }))
              }
              autoComplete="off"
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="query-description">
              {t("query_console.editor.description_label")}
            </Label>
            <Input
              id="query-description"
              value={draft.description}
              onChange={(event) =>
                setDraft((d) => ({ ...d, description: event.target.value }))
              }
              autoComplete="off"
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="query-sql">
              {t("query_console.editor.sql_label")}
            </Label>
            <Textarea
              id="query-sql"
              value={draft.sql}
              onChange={(event) =>
                setDraft((d) => ({ ...d, sql: event.target.value }))
              }
              className="min-h-40 font-mono text-xs"
              spellCheck={false}
              placeholder={t("query_console.editor.sql_placeholder")}
            />
          </div>

          {error ? (
            <Alert variant="destructive">
              <TriangleAlert />
              <AlertDescription>
                {apiErrorReason(error, t("query_console.editor.save_failed"))}
              </AlertDescription>
            </Alert>
          ) : null}

          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              onClick={() => onOpenChange(false)}
            >
              {t("query_console.editor.cancel")}
            </Button>
            <Button type="submit" disabled={!canSubmit}>
              {t("query_console.editor.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
