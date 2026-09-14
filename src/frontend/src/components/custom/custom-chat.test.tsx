vi.mock("@/api/custom-client");

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import type { ChatReply } from "@/api/custom-client";

import { CustomChat } from "./custom-chat";

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

beforeEach(() => {
  vi.resetAllMocks();
});

describe("<CustomChat>", () => {
  it("answers a one-time question with a table in the chat", async () => {
    vi.mocked(customClient.sendChat).mockResolvedValue({
      reply: "59 lines on 2026-09-01",
      result: { columns: ["day", "lines"], rows: [["2026-09-01", 59]] },
    });
    const onCreated = vi.fn();

    render(<CustomChat onCreated={onCreated} />, { wrapper });
    await userEvent.type(
      screen.getByRole("textbox"),
      "how many lines on the first?"
    );
    await userEvent.click(screen.getByRole("button", { name: /send/i }));

    expect(
      await screen.findByText("59 lines on 2026-09-01")
    ).toBeInTheDocument();
    expect(await screen.findByRole("cell", { name: "59" })).toBeInTheDocument();
    expect(onCreated).not.toHaveBeenCalled();
  });

  it("keeps a second question in the box while the first is still in flight", async () => {
    let answer: (reply: ChatReply) => void = () => {};
    vi.mocked(customClient.sendChat).mockImplementation(
      () =>
        new Promise<ChatReply>((resolve) => {
          answer = resolve;
        })
    );

    render(<CustomChat onCreated={vi.fn()} />, { wrapper });
    const box = screen.getByRole("textbox");
    await userEvent.type(box, "how many lines?{Enter}");
    // The send button is disabled now; Enter is not, and impatience is the
    // normal case in a chat.
    await userEvent.type(box, "and how many commits?{Enter}");

    expect(customClient.sendChat).toHaveBeenCalledTimes(1);
    expect(box).toHaveValue("and how many commits?");

    answer({ reply: "59" });
    expect(await screen.findByText("59")).toBeInTheDocument();
  });

  it("shows the reply and calls onCreated when a reply creates definitions", async () => {
    vi.mocked(customClient.sendChat).mockResolvedValue({
      reply: "Added a chart",
      created: { widgets: ["commits_graph"] },
    });
    const onCreated = vi.fn();

    render(<CustomChat onCreated={onCreated} />, { wrapper });
    await userEvent.type(screen.getByRole("textbox"), "chart commits per day");
    await userEvent.click(screen.getByRole("button", { name: /send/i }));

    expect(await screen.findByText("Added a chart")).toBeInTheDocument();
    expect(onCreated).toHaveBeenCalledWith({ widgets: ["commits_graph"] });
  });

  it("says which names it replaced", async () => {
    vi.mocked(customClient.sendChat).mockResolvedValue({
      reply: "Changed it",
      updated: { widgets: ["commits_table"], dashboard: "engineering" },
    });
    const onCreated = vi.fn();

    render(<CustomChat onCreated={onCreated} />, { wrapper });
    await userEvent.type(screen.getByRole("textbox"), "drop the author column");
    await userEvent.click(screen.getByRole("button", { name: /send/i }));

    expect(
      await screen.findByText(
        "Replaced widget commits_table · dashboard engineering"
      )
    ).toBeInTheDocument();
  });

  it("sends the thread so far, so a follow-up has its context", async () => {
    vi.mocked(customClient.sendChat)
      .mockResolvedValueOnce({ reply: "Here they are." })
      .mockResolvedValueOnce({ reply: "And by author." });
    const onCreated = vi.fn();

    render(<CustomChat onCreated={onCreated} />, { wrapper });
    const textbox = screen.getByRole("textbox");

    await userEvent.type(textbox, "lines per day?");
    await userEvent.click(screen.getByRole("button", { name: /send/i }));
    await screen.findByText("Here they are.");

    await userEvent.type(textbox, "and by author?");
    await userEvent.click(screen.getByRole("button", { name: /send/i }));
    await screen.findByText("And by author.");

    // The first call carries nothing; the second carries the turn before it.
    expect(customClient.sendChat).toHaveBeenNthCalledWith(1, "lines per day?", []);
    expect(customClient.sendChat).toHaveBeenNthCalledWith(2, "and by author?", [
      { role: "user", content: "lines per day?" },
      { role: "assistant", content: "Here they are." },
    ]);
  });

  it("keeps the typed question in the box when the send fails", async () => {
    vi.mocked(customClient.sendChat).mockRejectedValue(new Error("boom"));
    const onCreated = vi.fn();

    render(<CustomChat onCreated={onCreated} />, { wrapper });
    const textbox = screen.getByRole("textbox");
    await userEvent.type(textbox, "how many lines on the first?");
    await userEvent.click(screen.getByRole("button", { name: /send/i }));

    await screen.findByRole("alert");
    expect(textbox).toHaveValue("how many lines on the first?");
  });

  it("shows a readable message instead of the raw API error", async () => {
    vi.mocked(customClient.sendChat).mockRejectedValue(
      new Error("Custom API 500")
    );
    const onCreated = vi.fn();

    render(<CustomChat onCreated={onCreated} />, { wrapper });
    await userEvent.type(
      screen.getByRole("textbox"),
      "how many lines on the first?"
    );
    await userEvent.click(screen.getByRole("button", { name: /send/i }));

    const alert = await screen.findByRole("alert");
    expect(alert).not.toHaveTextContent("Custom API 500");
    expect(alert.textContent?.trim()).not.toBe("");
  });

  it("shows a pending affordance while the send is in flight", async () => {
    let resolveSend!: (reply: ChatReply) => void;
    vi.mocked(customClient.sendChat).mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveSend = resolve;
        })
    );
    const onCreated = vi.fn();

    render(<CustomChat onCreated={onCreated} />, { wrapper });
    await userEvent.type(
      screen.getByRole("textbox"),
      "how many lines on the first?"
    );
    await userEvent.click(screen.getByRole("button", { name: /send/i }));

    expect(
      await screen.findByRole("status", { name: /loading/i })
    ).toBeInTheDocument();

    resolveSend({ reply: "59 lines on 2026-09-01" });
    await screen.findByText("59 lines on 2026-09-01");
  });
});
