import { useSyncExternalStore } from "react";

export interface AssistantDraft {
  text: string;
}

let pending: AssistantDraft | null = null;

const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

// INVARIANT: the draft is held until the panel takes it, so it also reaches a
// panel that mounts after the call.
export function askAssistant(text: string): void {
  pending = { text };
  emit();
}

export function takeAssistantDraft(draft: AssistantDraft): void {
  if (pending !== draft) return;

  pending = null;
  emit();
}

export function useAssistantDraft(): AssistantDraft | null {
  return useSyncExternalStore(
    subscribe,
    () => pending,
    () => null
  );
}
