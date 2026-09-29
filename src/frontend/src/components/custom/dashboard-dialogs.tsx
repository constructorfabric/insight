import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { ConfirmDialog } from "@/components/confirm-dialog";
import { refusal } from "@/components/custom/refusal";
import { Input } from "@/components/ui/input";
import {
  invalidateDashboardList,
  useDuplicateDashboard,
  useRemoveDefinition,
  useRenameDefinition,
} from "@/queries/custom";

interface DialogProps {
  name: string;
  open: boolean;
  onClose: () => void;
}

function useRefusal(fallback: string) {
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);

  function refuse(failure: unknown) {
    setError(refusal(failure, fallback));
    void invalidateDashboardList(queryClient);
  }

  return { error, refuse, clear: () => setError(null) };
}

function NameDialog({
  open,
  title,
  description,
  confirmLabel,
  initial,
  fallback,
  pending,
  onSave,
  onClose,
}: {
  open: boolean;
  title: string;
  description: string;
  confirmLabel: string;
  initial: string;
  fallback: string;
  pending: boolean;
  onSave: (to: string) => Promise<unknown>;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const refused = useRefusal(fallback);

  const value = draft ?? initial;
  const to = value.trim();

  function close() {
    setDraft(null);
    refused.clear();
    onClose();
  }

  async function save() {
    if (to === "") return;

    try {
      await onSave(to);
      close();
    } catch (failure) {
      refused.refuse(failure);
    }
  }

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
      title={title}
      description={description}
      confirmLabel={confirmLabel}
      isPending={pending}
      confirmDisabled={to === ""}
      error={refused.error}
      onConfirm={() => void save()}
    >
      <Input
        autoFocus
        aria-label="Dashboard name"
        className="font-mono"
        readOnly={pending}
        value={value}
        onChange={(event) => {
          setDraft(event.target.value);
          refused.clear();
        }}
        onKeyDown={(event) => {
          if (event.key !== "Enter") return;
          event.preventDefault();
          void save();
        }}
      />
    </ConfirmDialog>
  );
}

export function RenameDashboard({
  name,
  open,
  onClose,
  onRenamed,
}: DialogProps & { onRenamed?: (to: string) => void }) {
  const rename = useRenameDefinition();

  async function save(to: string) {
    if (to === name) return;

    await rename.mutateAsync({ kind: "dashboards", name, to });
    onRenamed?.(to);
  }

  return (
    <NameDialog
      open={open}
      title="Rename dashboard"
      description={`Changes the name ${name} is stored under. Its title stays.`}
      confirmLabel="Rename"
      initial={name}
      fallback="The dashboard could not be renamed."
      pending={rename.isPending}
      onSave={save}
      onClose={onClose}
    />
  );
}

export function DuplicateDashboard({
  name,
  open,
  onClose,
  onDuplicated,
}: DialogProps & { onDuplicated?: (to: string) => void }) {
  const duplicate = useDuplicateDashboard();

  async function save(to: string) {
    const made = await duplicate.mutateAsync({ name, to });
    onDuplicated?.(made);
  }

  return (
    <NameDialog
      open={open}
      title="Duplicate dashboard"
      description={`Copies ${name} with its folder and tags.`}
      confirmLabel="Duplicate"
      initial={`${name}-copy`}
      fallback="The dashboard could not be duplicated."
      pending={duplicate.isPending}
      onSave={save}
      onClose={onClose}
    />
  );
}

export function DeleteDashboard({
  name,
  title,
  open,
  onClose,
  onDeleted,
}: DialogProps & { title: string; onDeleted?: () => void }) {
  const remove = useRemoveDefinition();
  const refused = useRefusal("The dashboard could not be deleted.");

  function close() {
    refused.clear();
    onClose();
  }

  async function confirm() {
    try {
      await remove.mutateAsync({ kind: "dashboards", name });
      close();
      onDeleted?.();
    } catch (failure) {
      refused.refuse(failure);
    }
  }

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
      title={`Delete ${title}?`}
      confirmLabel="Delete dashboard"
      destructive
      isPending={remove.isPending}
      error={refused.error}
      onConfirm={() => void confirm()}
    />
  );
}
