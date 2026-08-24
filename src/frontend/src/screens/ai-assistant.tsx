/**
 * AI assistant settings as the portal's Manage zone renders them: the caller's
 * key, the context both scopes contribute to every explanation, and — for
 * admins — the system prompt the tenant runs.
 *
 * The Manage entry is hidden where the deployment does not offer explanations,
 * so the "not enabled" state is reached only by a bookmark or a stale tab.
 */
import { Lock, Plus, Sparkles, Trash2 } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { useIsAdmin } from "@/queries/identity-me";
import {
  useAiConfig,
  useAiContext,
  useAiCredentialStatus,
  useAiSettings,
  useCreateAiContext,
  useDeleteAiContext,
  useForgetAiCredential,
  useResetAiSystemPrompt,
  useSaveAiCredential,
  useSaveAiSystemPrompt,
} from "@/queries/ai";
import type { ContextEntry, ContextScope } from "@/api/ai-client";
import { TEXT_LABEL, TEXT_TITLE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export function AiAssistantBody() {
  const config = useAiConfig();
  const featureOn = config.data?.enabled === true;
  const standKey = config.data?.stand_key === true;
  const { isAdmin } = useIsAdmin();

  if (config.isPending) {
    return <Shell>Loading…</Shell>;
  }

  // A failed check is not the same answer as "switched off" — saying "off"
  // here would send an admin to the chart to fix a network blip.
  if (config.isError) {
    return (
      <Shell>
        <Card>
          <CardHeader>
            <CardTitle>Could not check whether AI is available</CardTitle>
            <CardDescription>
              The request failed, so this page cannot tell whether explanations
              are switched on here.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <Button size="sm" variant="outline" onClick={() => void config.refetch()}>
              Try again
            </Button>
          </CardContent>
        </Card>
      </Shell>
    );
  }

  if (!featureOn) {
    return (
      <Shell>
        <Card>
          <CardHeader>
            <CardTitle>AI explanations are off here</CardTitle>
            <CardDescription>
              This deployment does not offer AI explanations. An operator turns
              them on in the chart.
            </CardDescription>
          </CardHeader>
        </Card>
      </Shell>
    );
  }

  return (
    <Shell>
      {standKey ? <StandKeyCard /> : <KeyCard />}
      {isAdmin ? <SystemPromptCard /> : null}
      <ContextCard
        scope="person"
        title="Your context"
        description="Only you can read or edit these. They shape your explanations only."
        canWrite
      />
      <ContextCard
        scope="tenant"
        title="Organisation context"
        description="Read into every explanation in this organisation."
        canWrite={isAdmin}
      />
    </Shell>
  );
}

function StandKeyCard() {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Anthropic key</CardTitle>
        <CardDescription>
          One key for the whole stand, set by an administrator.
        </CardDescription>
      </CardHeader>
    </Card>
  );
}

function Shell({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex flex-1 flex-col gap-6 p-4 md:p-6">
      <header className="flex items-center gap-2">
        <Sparkles className="size-4" aria-hidden />
        <h2 className={TEXT_TITLE}>AI assistant</h2>
      </header>
      <div className="flex max-w-3xl flex-col gap-4">{children}</div>
    </div>
  );
}

function KeyCard() {
  const status = useAiCredentialStatus(true);
  const save = useSaveAiCredential();
  const forget = useForgetAiCredential();
  const [token, setToken] = useState("");

  const configured = status.data?.configured === true;

  return (
    <Card>
      <CardHeader>
        <CardTitle>Anthropic API key</CardTitle>
        <CardDescription>
          Your key, your usage. Stored encrypted, and shown only as its last
          four characters once saved.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {configured ? (
          <div className="flex flex-wrap items-center gap-2">
            <code className="bg-muted rounded-md px-2 py-1 font-mono text-xs">
              sk-ant-••••••••••••{status.data?.hint}
            </code>
            <Button
              size="sm"
              variant="outline"
              onClick={() => forget.mutate()}
              disabled={forget.isPending}
            >
              Remove
            </Button>
          </div>
        ) : null}
        <div className="flex flex-wrap items-center gap-2">
          <Input
            type="password"
            value={token}
            placeholder={configured ? "Replace with a new key" : "sk-ant-…"}
            onChange={(event) => setToken(event.target.value)}
            className="max-w-sm"
            aria-label="Anthropic API key"
          />
          <Button
            size="sm"
            onClick={() =>
              save.mutate(token, { onSuccess: () => setToken("") })
            }
            disabled={token.trim().length === 0 || save.isPending}
          >
            {configured ? "Replace" : "Save"}
          </Button>
        </div>
        {save.isError ? (
          <p className={cn(TEXT_LABEL, "text-destructive")}>
            That key was not accepted. Check it and try again.
          </p>
        ) : null}
      </CardContent>
    </Card>
  );
}

function SystemPromptCard() {
  const settings = useAiSettings(true);
  const save = useSaveAiSystemPrompt();
  const reset = useResetAiSystemPrompt();
  // Null means "whatever the server holds" — the stored prompt arrives after
  // the first render, and seeding state from it in an effect would overwrite
  // an edit made while the read was still in flight.
  const [draft, setDraft] = useState<string | null>(null);

  const stored = settings.data?.system_prompt ?? "";
  const text = draft ?? stored;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center gap-2">
          System prompt
          <span
            className={cn(
              TEXT_LABEL,
              "flex items-center gap-1 rounded-full border px-2 py-0.5"
            )}
          >
            <Lock className="size-3" aria-hidden />
            Admins only
          </span>
        </CardTitle>
        <CardDescription>
          What Claude is told before anything else, for everyone in this
          organisation.{" "}
          {settings.data?.is_default
            ? "Currently the shipped default."
            : "Edited for this organisation."}
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <Textarea
          value={text}
          rows={10}
          onChange={(event) => setDraft(event.target.value)}
          aria-label="System prompt"
        />
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            onClick={() =>
              save.mutate(text, { onSuccess: () => setDraft(null) })
            }
            disabled={text.trim().length === 0 || save.isPending}
          >
            Save
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => reset.mutate(undefined, { onSuccess: () => setDraft(null) })}
            // Only once the read confirms a tenant prompt exists: enabled on an
            // unknown state, a mis-click would delete a prompt nobody has seen.
            disabled={reset.isPending || settings.data?.is_default !== false}
          >
            Reset to default
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function ContextCard({
  scope,
  title,
  description,
  canWrite,
}: {
  scope: ContextScope;
  title: string;
  description: string;
  canWrite: boolean;
}) {
  const entries = useAiContext(true);
  const create = useCreateAiContext();
  const remove = useDeleteAiContext();
  const [adding, setAdding] = useState(false);
  const [draftTitle, setDraftTitle] = useState("");
  const [draftBody, setDraftBody] = useState("");

  const mine: ContextEntry[] = (entries.data ?? []).filter(
    (entry) => entry.scope === scope
  );

  const add = () => {
    create.mutate(
      { scope, title: draftTitle, body: draftBody },
      {
        onSuccess: () => {
          setDraftTitle("");
          setDraftBody("");
          setAdding(false);
        },
      }
    );
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center gap-2">
          {title}
          {canWrite ? null : (
            <span
              className={cn(
                TEXT_LABEL,
                "flex items-center gap-1 rounded-full border px-2 py-0.5"
              )}
            >
              <Lock className="size-3" aria-hidden />
              Admins write
            </span>
          )}
        </CardTitle>
        <CardDescription>{description}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {mine.length === 0 ? (
          <p className={TEXT_LABEL}>Nothing written yet.</p>
        ) : (
          <ul className="flex flex-col gap-2">
            {mine.map((entry) => (
              <li
                key={entry.id}
                className="flex items-start justify-between gap-3 rounded-md border p-3"
              >
                <div className="min-w-0">
                  <p className="text-sm font-medium">{entry.title}</p>
                  <p className={cn(TEXT_LABEL, "font-normal whitespace-pre-line")}>
                    {entry.body}
                  </p>
                </div>
                {canWrite ? (
                  <Button
                    size="sm"
                    variant="ghost"
                    aria-label={`Delete ${entry.title}`}
                    onClick={() => remove.mutate(entry.id)}
                  >
                    <Trash2 className="size-4" aria-hidden />
                  </Button>
                ) : null}
              </li>
            ))}
          </ul>
        )}

        {canWrite && adding ? (
          <div className="flex flex-col gap-2 rounded-md border p-3">
            <Input
              value={draftTitle}
              placeholder="Title"
              onChange={(event) => setDraftTitle(event.target.value)}
              aria-label={`${title} entry title`}
            />
            <Textarea
              value={draftBody}
              rows={4}
              placeholder="What should the model know?"
              onChange={(event) => setDraftBody(event.target.value)}
              aria-label={`${title} entry body`}
            />
            <div className="flex gap-2">
              <Button
                size="sm"
                onClick={add}
                disabled={
                  draftTitle.trim().length === 0 ||
                  draftBody.trim().length === 0 ||
                  create.isPending
                }
              >
                Add
              </Button>
              <Button size="sm" variant="outline" onClick={() => setAdding(false)}>
                Cancel
              </Button>
            </div>
          </div>
        ) : null}

        {canWrite && !adding ? (
          <div>
            <Button size="sm" variant="outline" onClick={() => setAdding(true)}>
              <Plus className="size-4" aria-hidden />
              Add entry
            </Button>
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}
