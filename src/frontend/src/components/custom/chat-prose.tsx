import type { ReactNode } from "react";

/**
 * The model's reply as prose: its line breaks kept, and `**bold**` shown as
 * bold rather than as asterisks.
 *
 * The prompt asks for plain text, and the panel is not a markdown surface —
 * but a model emphasises things anyway, and raw asterisks in the middle of a
 * sentence read as a bug. Emphasis is the whole vocabulary here; anything
 * else it types is shown as typed.
 */
export function ChatProse({ text }: { text: string }) {
  return <>{text.split(BOLD).map(toSegment)}</>;
}

/** Capturing, so `split` keeps the emphasised words as their own entries. */
const BOLD = /\*\*([^*]+)\*\*/g;

function toSegment(part: string, index: number): ReactNode {
  // Odd entries are the capture groups: what sat between the asterisks.
  return index % 2 === 1 ? <strong key={index}>{part}</strong> : part;
}
