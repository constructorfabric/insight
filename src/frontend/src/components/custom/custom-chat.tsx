import { useEffect, useRef, useState } from "react";
import { SendHorizontal, Sparkles } from "lucide-react";

import { cn } from "@/lib/utils";

import type { ChatCreated, ChatReply } from "@/api/custom-client";
import { ChatProse } from "@/components/custom/chat-prose";
import { CustomTable } from "@/components/custom/custom-table";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import { useSendChat } from "@/queries/custom";
import { TEXT_BODY, TEXT_HEADING, TEXT_LABEL } from "@/lib/type-scale";

export interface CustomChatProps {
  onCreated: (created: ChatCreated) => void;
}

interface Exchange {
  id: number;
  question: string;
  reply?: ChatReply;
  error?: string;
}

/**
 * Which tool the model called, read from the shape of what came back: names
 * mean it built, anything else answered. Never inferred from the prose.
 *
 * Keyed off `created`, not `result`: an answer about what data exists runs no
 * query, so a reply without rows is still an answer.
 */
function toolUsed(reply: ChatReply): string {
  return reply.created || reply.updated ? "create" : "answer";
}

/** A creation set read out as prose: `metric x · widget y`. */
function names(set: ChatCreated | undefined): string[] {
  return [
    set?.metric ? `metric ${set.metric}` : null,
    set?.widgets.length
      ? `${set.widgets.length === 1 ? "widget" : "widgets"} ${set.widgets.join(", ")}`
      : null,
    set?.dashboard ? `dashboard ${set.dashboard}` : null,
  ].filter((entry): entry is string => entry !== null);
}

export function CustomChat({ onCreated }: CustomChatProps) {
  const [message, setMessage] = useState("");
  const [exchanges, setExchanges] = useState<Exchange[]>([]);
  const sendChat = useSendChat();
  const threadEnd = useRef<HTMLDivElement | null>(null);

  // A thread that grows below the fold reads as a dead panel.
  useEffect(() => {
    // Optional call: jsdom does not implement it, and a thread that cannot
    // scroll is still a thread.
    threadEnd.current?.scrollIntoView?.({ block: "end" });
  }, [exchanges]);

  async function handleSend() {
    const question = message.trim();
    // The send button is disabled while one is in flight; Enter is not, so
    // the check lives here rather than beside the button.
    if (!question || sendChat.isPending) return;

    const id = Date.now() + Math.random();
    const history = exchanges.flatMap((exchange) =>
      exchange.reply
        ? [
            { role: "user" as const, content: exchange.question },
            { role: "assistant" as const, content: exchange.reply.reply },
          ]
        : []
    );
    setMessage("");
    setExchanges((prev) => [...prev, { id, question }]);

    try {
      const reply = await sendChat.mutateAsync({ message: question, history });
      setExchanges((prev) =>
        prev.map((exchange) =>
          exchange.id === id ? { ...exchange, reply } : exchange
        )
      );
      if (reply.created) onCreated(reply.created);
    } catch {
      setMessage(question);
      setExchanges((prev) =>
        prev.map((exchange) =>
          exchange.id === id
            ? { ...exchange, error: "The chat request failed." }
            : exchange
        )
      );
    }
  }

  return (
    <aside className="flex h-full min-h-0 flex-col border-s bg-sidebar">
      <header className="flex items-center gap-2 border-b px-4 py-3">
        <Sparkles className="size-4 text-muted-foreground" aria-hidden />
        <h2 className={TEXT_HEADING}>Assistant</h2>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
        {exchanges.length === 0 ? (
          <p className={cn(TEXT_LABEL, "leading-relaxed")}>
            Ask a question about the data, or describe a dashboard to build.
          </p>
        ) : (
          <ul className="flex flex-col gap-4">
            {exchanges.map((exchange) => (
              <li key={exchange.id} className="flex flex-col gap-2">
                <p
                  className={cn(
                    TEXT_BODY,
                    "ms-auto max-w-[85%] rounded-2xl rounded-ee-sm bg-primary px-3 py-2 whitespace-pre-line text-primary-foreground"
                  )}
                >
                  {exchange.question}
                </p>

                {exchange.reply ? (
                  <ChatAnswer reply={exchange.reply} />
                ) : exchange.error ? (
                  <p
                    role="alert"
                    className={cn(
                      TEXT_BODY,
                      "me-auto max-w-[85%] rounded-2xl rounded-es-sm bg-destructive/10 px-3 py-2 text-destructive"
                    )}
                  >
                    {exchange.error}
                  </p>
                ) : (
                  <span
                    className={cn(
                      TEXT_LABEL,
                      "me-auto flex items-center gap-2 px-1"
                    )}
                  >
                    <Spinner className="size-3" /> Thinking…
                  </span>
                )}
              </li>
            ))}
          </ul>
        )}
        <div ref={threadEnd} />
      </div>

      {/* The composer, shaped like every chat the reader already uses: the box
          and the send action on one row, the action an icon. A labelled block
          button below the box read as a form to submit rather than a message
          to send, and the keyboard hint beside it said nothing after the
          first send. */}
      <div className="flex items-center gap-2 border-t p-3">
        <Textarea
          data-testid="chat-input"
          value={message}
          rows={2}
          className="max-h-40 min-h-11 flex-1 resize-none bg-background"
          onChange={(event) => setMessage(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              void handleSend();
            }
          }}
          placeholder="Message"
        />
        <Button
          data-testid="chat-send"
          type="button"
          variant="ghost"
          size="icon"
          aria-label="Send"
          // `size="icon"` is 50x36 here: a wide box around a plane, 8px
          // shorter than the input beside it. Square it and the two line up.
          className="size-10 shrink-0 text-primary disabled:opacity-40"
          onClick={() => void handleSend()}
          disabled={sendChat.isPending || message.trim() === ""}
        >
          <SendHorizontal className="size-5 fill-current stroke-1" />
        </Button>
      </div>
    </aside>
  );
}

function ChatAnswer({ reply }: { reply: ChatReply }) {
  const tool = toolUsed(reply);
  const built = names(reply.created);
  const replaced = names(reply.updated);

  return (
    <div className="me-auto flex w-full flex-col gap-2 rounded-2xl rounded-es-sm bg-background px-3 py-2 shadow-xs">
      <Badge variant="secondary" className="w-fit font-mono">
        {tool}
      </Badge>

      <p className={cn(TEXT_BODY, "leading-relaxed whitespace-pre-line")}>
        <ChatProse text={reply.reply} />
      </p>

      {/* A result with no rows says nothing a sentence has not already said. */}
      {reply.result && reply.result.rows.length > 0 ? (
        <div className="max-h-64 overflow-auto rounded-md border">
          <CustomTable result={reply.result} />
        </div>
      ) : null}

      {built.length ? (
        <p className={TEXT_LABEL}>Built {built.join(" · ")}</p>
      ) : null}

      {replaced.length ? (
        <p className={TEXT_LABEL}>Replaced {replaced.join(" · ")}</p>
      ) : null}
    </div>
  );
}
