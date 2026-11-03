import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { DEFINITION_NAME_MAX } from "@/api/custom-client";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { refusal } from "@/components/custom/refusal";
import { Input } from "@/components/ui/input";
import {
  invalidateDashboardList,
  useDuplicateDashboard,
  useRemoveDefinition,
  useRenameDashboard,
} from "@/queries/custom";

const COPY_SUFFIX = " copy";

function copyName(name: string): string {
  const kept = name.slice(0, DEFINITION_NAME_MAX - COPY_SUFFIX.length);

  return `${kept.trimEnd()}${COPY_SUFFIX}`;
}

interface DialogProps {
  id: string;
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
  description?: string;
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
        maxLength={DEFINITION_NAME_MAX}
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

export function RenameDashboard({ id, name, open, onClose }: DialogProps) {
  const rename = useRenameDashboard();

  async function save(to: string) {
    if (to === name) return;

    await rename.mutateAsync({ id, to });
  }

  return (
    <NameDialog
      open={open}
      title="Rename dashboard"
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
  id,
  name,
  open,
  onClose,
  onDuplicated,
}: DialogProps & { onDuplicated?: (id: string) => void }) {
  const duplicate = useDuplicateDashboard();

  async function save(to: string) {
    const made = await duplicate.mutateAsync({ id, to });
    onDuplicated?.(made);
  }

  return (
    <NameDialog
      open={open}
      title="Duplicate dashboard"
      description={`Copies ${name}.`}
      confirmLabel="Duplicate"
      initial={copyName(name)}
      fallback="The dashboard could not be duplicated."
      pending={duplicate.isPending}
      onSave={save}
      onClose={onClose}
    />
  );
}

export function DeleteDashboard({
  id,
  title,
  open,
  onClose,
  onDeleted,
}: Omit<DialogProps, "name"> & { title: string; onDeleted?: () => void }) {
  const remove = useRemoveDefinition();
  const refused = useRefusal("The dashboard could not be deleted.");

  function close() {
    refused.clear();
    onClose();
  }

  async function confirm() {
    try {
      await remove.mutateAsync({ kind: "dashboards", id });
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
      description="It is removed for everyone it is shared with, and from every folder holding it. This can't be undone."
      confirmLabel="Delete dashboard"
      destructive
      isPending={remove.isPending}
      error={refused.error}
      onConfirm={() => void confirm()}
    />
  );
}
