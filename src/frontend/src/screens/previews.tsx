/**
 * Preview experiments — self-service list / create / delete, behind
 * {@link usePreviewsGate}. Bookmarks land here past the hidden nav entry, so
 * the screen refuses on its own; the server enforces the same roles anyway.
 */
import { ExternalLink, Plus, TriangleAlert } from "lucide-react";
import { useState } from "react";

import type { Experiment } from "@/api/previews-client";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SidebarTrigger } from "@/components/ui/sidebar";
import { Spinner } from "@/components/ui/spinner";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  useCreateExperiment,
  useDeleteExperiment,
  useExperiments,
  usePreviewsGate,
} from "@/queries/previews";

export function PreviewsScreen() {
  return (
    <>
      <header className="sticky top-0 z-20 flex items-center gap-3 border-b bg-background/95 px-4 py-3 backdrop-blur-sm">
        <SidebarTrigger />
        <h1 className="text-lg font-semibold tracking-tight">Previews</h1>
      </header>

      <PreviewsBody />
    </>
  );
}

/** The previews console alone — the portal's Manage zone brings its own chrome. */
export function PreviewsBody() {
  const allowed = usePreviewsGate();
  if (!allowed) {
    return (
      <div className="mx-auto w-full max-w-md p-8" role="alert">
        <div className="rounded-lg border p-6 text-center">
          <p className="text-sm font-semibold">Previews are not available</p>
          <p className="mt-2 text-sm text-muted-foreground">
            Managing preview experiments needs the previews-admin or admin
            role, on a stand with experiments enabled.
          </p>
        </div>
      </div>
    );
  }
  return (
    <main className="flex flex-1 flex-col gap-6 p-4 md:p-6">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-6 pb-12">
        <section>
          <h2 className="text-lg font-semibold tracking-tight">
            Preview experiments
          </h2>
          <p className="text-sm text-muted-foreground">
            Each experiment serves one frontend build under /exp/&lt;name&gt;
            on the preview host, against this stand&apos;s data.
          </p>
        </section>
        <CreateExperimentForm />
        <ExperimentList />
      </div>
    </main>
  );
}

/** The server's problem detail when it says one, else a generic line. */
function errorMessage(error: unknown): string {
  if (error && typeof error === "object") {
    const body = (error as { body?: { detail?: string } }).body;
    if (typeof body?.detail === "string" && body.detail) return body.detail;
    if (error instanceof Error) return error.message;
  }
  return "The request failed";
}

function CreateExperimentForm() {
  const [name, setName] = useState("");
  const [tag, setTag] = useState("");
  const create = useCreateExperiment();

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (!name.trim() || !tag.trim() || create.isPending) return;
    create.mutate(
      { name: name.trim(), tag: tag.trim() },
      {
        onSuccess: () => {
          setName("");
          setTag("");
        },
      },
    );
  };

  return (
    <form
      onSubmit={submit}
      className="flex flex-col gap-3 rounded-lg border bg-card p-4"
      aria-label="Create a preview experiment"
    >
      <div className="grid gap-3 sm:grid-cols-[1fr_1fr_auto] sm:items-end">
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="previews-name">Name</Label>
          <Input
            id="previews-name"
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="my-experiment"
            autoComplete="off"
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="previews-tag">Image tag</Label>
          <Input
            id="previews-tag"
            value={tag}
            onChange={(event) => setTag(event.target.value)}
            placeholder="preview-my-branch"
            autoComplete="off"
          />
        </div>
        <Button
          type="submit"
          disabled={!name.trim() || !tag.trim() || create.isPending}
        >
          {create.isPending ? <Spinner /> : <Plus />}
          Create
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        The server validates both: the name is a DNS label (at most 55
        characters), the tag a preview- or CI build tag.
      </p>
      {create.isError ? (
        <Alert variant="destructive">
          <TriangleAlert />
          <AlertDescription>{errorMessage(create.error)}</AlertDescription>
        </Alert>
      ) : null}
    </form>
  );
}

function ExperimentList() {
  const experiments = useExperiments();
  const [deleting, setDeleting] = useState<Experiment | null>(null);
  const remove = useDeleteExperiment();

  if (experiments.isPending) {
    return (
      <div className="flex justify-center p-8">
        <Spinner />
      </div>
    );
  }
  if (experiments.isError) {
    return (
      <Alert variant="destructive">
        <TriangleAlert />
        <AlertDescription>
          Could not list the experiments.{" "}
          <button
            type="button"
            className="underline"
            onClick={() => void experiments.refetch()}
          >
            Retry
          </button>
        </AlertDescription>
      </Alert>
    );
  }
  if (experiments.data.length === 0) {
    return (
      <p className="rounded-lg border p-6 text-center text-sm text-muted-foreground">
        No experiments are running.
      </p>
    );
  }

  return (
    <div className="overflow-x-auto rounded-lg border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Name</TableHead>
            <TableHead>Tag</TableHead>
            <TableHead>Status</TableHead>
            <TableHead>Expires</TableHead>
            <TableHead className="text-right">Actions</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {experiments.data.map((experiment) => (
            <TableRow key={experiment.name}>
              <TableCell className="font-mono text-xs">
                {experiment.name}
              </TableCell>
              <TableCell className="font-mono text-xs">
                {experiment.tag}
              </TableCell>
              <TableCell className="text-muted-foreground">
                {experiment.status}
              </TableCell>
              <TableCell className="text-muted-foreground">
                {experiment.expiresAt
                  ? new Date(experiment.expiresAt).toLocaleString()
                  : "—"}
              </TableCell>
              <TableCell className="text-right">
                <div className="flex justify-end gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    render={
                      // Cross-origin (the preview host), so a plain anchor.
                      <a
                        href={experiment.url}
                        target="_blank"
                        rel="noreferrer"
                        aria-label={`Open experiment ${experiment.name}`}
                      />
                    }
                  >
                    <ExternalLink />
                    Open
                  </Button>
                  <Button
                    variant="destructive"
                    size="sm"
                    onClick={() => {
                      remove.reset();
                      setDeleting(experiment);
                    }}
                  >
                    Delete
                  </Button>
                </div>
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
      <ConfirmDialog
        open={deleting != null}
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
        title="Delete this experiment?"
        description={
          deleting
            ? `${deleting.name} stops serving immediately; the build itself is untouched.`
            : undefined
        }
        confirmLabel="Delete"
        destructive
        isPending={remove.isPending}
        error={remove.isError ? errorMessage(remove.error) : null}
        onConfirm={() => {
          if (!deleting) return;
          remove.mutate(deleting.name, {
            onSuccess: () => setDeleting(null),
          });
        }}
      />
    </div>
  );
}
