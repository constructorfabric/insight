// @vitest-environment jsdom
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

const hooks = vi.hoisted(() => ({
  toast: { success: vi.fn(), error: vi.fn() },
  submit: {
    mutate: vi.fn(),
    reset: vi.fn(),
    isPending: false,
    error: null as unknown,
  },
  screen: "/portal/overview",
}));

vi.mock("@/components/ui/sonner", () => ({ toast: hooks.toast }));
vi.mock("@/queries/feedback", () => ({ useSubmitFeedback: () => hooks.submit }));
vi.mock("@/telemetry", () => ({
  APP_NAME: "insight-frontend",
  APP_VERSION: "1.2.3",
  currentScreen: () => hooks.screen,
}));

import { FEEDBACK_MESSAGE_MAX } from "@/api/feedback-client";
import { FeedbackDialog } from "./feedback-dialog";

function sendButton(): HTMLElement {
  return screen.getByRole("button", { name: "Send" });
}

beforeEach(() => {
  hooks.submit.mutate.mockReset();
  hooks.submit.reset.mockReset();
  hooks.submit.isPending = false;
  hooks.submit.error = null;
  hooks.toast.success.mockReset();
  hooks.screen = "/portal/overview";
});

describe("FeedbackDialog", () => {
  it("will not send an empty message", async () => {
    render(<FeedbackDialog open onOpenChange={() => {}} />);

    expect(sendButton()).toBeDisabled();

    await userEvent.type(screen.getByLabelText("Your feedback"), "   ");

    expect(sendButton()).toBeDisabled();
  });

  it("sends the screen the sender was on with what they wrote", async () => {
    render(<FeedbackDialog open onOpenChange={() => {}} />);

    await userEvent.type(
      screen.getByLabelText("Your feedback"),
      "the chart is empty",
    );
    await userEvent.click(sendButton());

    expect(hooks.submit.mutate).toHaveBeenCalledWith(
      {
        message: "the chart is empty",
        path: "/portal/overview",
        app_name: "insight-frontend",
        app_version: "1.2.3",
      },
      expect.anything(),
    );
  });

  it("clears the form once a send lands, so the next one starts empty", async () => {
    hooks.submit.mutate.mockImplementation(
      (_body: unknown, options: { onSuccess: () => void }) => options.onSuccess(),
    );
    const onOpenChange = vi.fn();
    render(<FeedbackDialog open onOpenChange={onOpenChange} />);

    const message = screen.getByLabelText("Your feedback");
    await userEvent.type(message, "add an export button");
    await userEvent.click(sendButton());

    expect(hooks.toast.success).toHaveBeenCalled();
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(message).toHaveValue("");
  });

  it("stops the box at the longest message the service stores", async () => {
    render(<FeedbackDialog open onOpenChange={() => {}} />);

    const message = screen.getByLabelText("Your feedback");
    await userEvent.click(message);
    await userEvent.paste("x".repeat(FEEDBACK_MESSAGE_MAX + 50));

    expect(message).toHaveValue("x".repeat(FEEDBACK_MESSAGE_MAX));
  });

  it("keeps the box inside the dialog however much is written", () => {
    render(<FeedbackDialog open onOpenChange={() => {}} />);

    expect(screen.getByLabelText("Your feedback")).toHaveClass(
      "max-h-[40vh]",
      "overflow-y-auto",
    );
  });

  it("counts what is written against the budget", async () => {
    render(<FeedbackDialog open onOpenChange={() => {}} />);

    await userEvent.type(screen.getByLabelText("Your feedback"), "empty");

    expect(
      screen.getByText(`5 / ${FEEDBACK_MESSAGE_MAX}`),
    ).toBeInTheDocument();
  });

  it("announces the budget to whoever cannot see the counter", () => {
    render(<FeedbackDialog open onOpenChange={() => {}} />);

    expect(screen.getByLabelText("Your feedback")).toHaveAccessibleDescription(
      `0 / ${FEEDBACK_MESSAGE_MAX}`,
    );
  });

  it("keeps the dialog open on a refusal, and says why", () => {
    hooks.submit.error = { name: "AnalyticsApiError" };
    render(<FeedbackDialog open onOpenChange={() => {}} />);

    expect(
      screen.getByText("Could not send your feedback. Try again."),
    ).toBeInTheDocument();
  });
});
