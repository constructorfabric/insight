// @vitest-environment jsdom
/**
 * What the Manage-zone surface may show, and to whom. The key is the sharp
 * case: it goes out in a request and comes back only as four characters.
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const state = vi.hoisted(() => ({
  config: { enabled: true, model: "claude-sonnet-5" } as
    | {
        enabled: boolean;
        model: string;
        stand_key?: boolean;
        admin_only?: boolean;
      }
    | undefined,
  configPending: false,
  isAdmin: true,
  credential: { configured: true, hint: "wxyz" },
  settings: { system_prompt: "SHIPPED", is_default: true },
  entries: [
    {
      id: "p1",
      scope: "person" as const,
      title: "How my week runs",
      body: "Meeting-heavy midweek.",
      updated_at: "now",
    },
    {
      id: "t1",
      scope: "tenant" as const,
      title: "How we read metrics",
      body: "Systems, not people.",
      updated_at: "now",
    },
  ],
  saveCredential: vi.fn(),
  forgetCredential: vi.fn(),
  savePrompt: vi.fn(),
  resetPrompt: vi.fn(),
  createEntry: vi.fn(),
  deleteEntry: vi.fn(),
}));

vi.mock("@/queries/identity-me", () => ({
  useIsAdmin: () => ({
    isAdmin: state.isAdmin,
    isPending: false,
    isError: false,
    retry: () => {},
  }),
}));

vi.mock("@/queries/ai", () => ({
  useAiConfig: () => ({ data: state.config, isPending: state.configPending }),
  useAiCredentialStatus: () => ({ data: state.credential }),
  useAiSettings: () => ({ data: state.settings }),
  useAiContext: () => ({ data: state.entries }),
  useSaveAiCredential: () => ({
    mutate: state.saveCredential,
    isPending: false,
    isError: false,
  }),
  useForgetAiCredential: () => ({
    mutate: state.forgetCredential,
    isPending: false,
  }),
  useSaveAiSystemPrompt: () => ({ mutate: state.savePrompt, isPending: false }),
  useResetAiSystemPrompt: () => ({
    mutate: state.resetPrompt,
    isPending: false,
  }),
  useCreateAiContext: () => ({ mutate: state.createEntry, isPending: false }),
  useDeleteAiContext: () => ({ mutate: state.deleteEntry, isPending: false }),
  useUpdateAiContext: () => ({ mutate: vi.fn(), isPending: false }),
}));

import { AiAssistantBody } from "./ai-assistant";

beforeEach(() => {
  state.config = { enabled: true, model: "claude-sonnet-5" };
  state.configPending = false;
  state.isAdmin = true;
  state.credential = { configured: true, hint: "wxyz" };
  state.settings = { system_prompt: "SHIPPED", is_default: true };
  state.saveCredential.mockClear();
  state.forgetCredential.mockClear();
  state.savePrompt.mockClear();
  state.resetPrompt.mockClear();
  state.createEntry.mockClear();
  state.deleteEntry.mockClear();
});

describe("AiAssistantBody", () => {
  it("says the deployment does not offer explanations rather than pretending", () => {
    state.config = { enabled: false, model: "" };

    render(<AiAssistantBody />);

    expect(screen.getByText("AI explanations are off here")).toBeInTheDocument();
    expect(screen.queryByLabelText("Anthropic API key")).toBeNull();
  });

  it("shows only the last four characters of a stored key", () => {
    render(<AiAssistantBody />);

    expect(screen.getByText(/wxyz/)).toBeInTheDocument();
    expect(screen.queryByText(/sk-ant-api/)).toBeNull();
  });

  it("keeps Save inert until something has been typed", async () => {
    const user = userEvent.setup();
    render(<AiAssistantBody />);

    const save = screen.getByRole("button", { name: "Replace" });
    expect(save).toBeDisabled();

    await user.type(screen.getByLabelText("Anthropic API key"), "sk-ant-new");
    expect(save).toBeEnabled();
    await user.click(save);

    expect(state.saveCredential).toHaveBeenCalledWith(
      "sk-ant-new",
      expect.anything()
    );
  });

  it("offers to forget a stored key", async () => {
    const user = userEvent.setup();
    render(<AiAssistantBody />);

    await user.click(screen.getByRole("button", { name: "Remove" }));

    expect(state.forgetCredential).toHaveBeenCalled();
  });

  it("hides the system prompt from a non-admin", () => {
    state.isAdmin = false;

    render(<AiAssistantBody />);

    expect(screen.queryByLabelText("System prompt")).toBeNull();
    expect(screen.getByText("Your context")).toBeInTheDocument();
  });

  it("lets an admin write the prompt and says it is the shipped one", async () => {
    const user = userEvent.setup();
    render(<AiAssistantBody />);

    expect(screen.getByText(/Currently the shipped default/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reset to default" })).toBeDisabled();

    await user.type(screen.getByLabelText("System prompt"), " plus ours");
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(state.savePrompt).toHaveBeenCalled();
  });

  it("offers a reset once the tenant has its own prompt", () => {
    state.settings = { system_prompt: "OURS", is_default: false };

    render(<AiAssistantBody />);

    expect(screen.getByText(/Edited for this organisation/)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Reset to default" })
    ).toBeEnabled();
  });

  it("splits the entries by the scope that owns them", () => {
    render(<AiAssistantBody />);

    expect(screen.getByText("How my week runs")).toBeInTheDocument();
    expect(screen.getByText("How we read metrics")).toBeInTheDocument();
  });

  it("adds an entry to the scope its form belongs to", async () => {
    const user = userEvent.setup();
    render(<AiAssistantBody />);

    await user.click(screen.getAllByRole("button", { name: /Add entry/ })[0]);
    await user.type(screen.getByLabelText("Your context entry title"), "Title");
    await user.type(screen.getByLabelText("Your context entry body"), "Body");
    await user.click(screen.getByRole("button", { name: "Add" }));

    expect(state.createEntry).toHaveBeenCalledWith(
      { scope: "person", title: "Title", body: "Body" },
      expect.anything()
    );
  });

  it("removes an entry the reader owns", async () => {
    const user = userEvent.setup();
    render(<AiAssistantBody />);

    await user.click(
      screen.getByRole("button", { name: "Delete How my week runs" })
    );

    expect(state.deleteEntry).toHaveBeenCalledWith("p1");
  });

  it("offers nowhere to paste a key when the stand supplies one", () => {
    state.config = {
      enabled: true,
      model: "claude-sonnet-5",
      stand_key: true,
    };

    render(<AiAssistantBody />);

    // Said, not left blank: someone who came to paste a key needs the reason.
    expect(
      screen.getByText("One key for the whole stand, set by an administrator.")
    ).toBeInTheDocument();
    expect(screen.queryByLabelText(/Anthropic key/i)).toBeNull();
  });

  it("leaves organisation entries read-only for a non-admin", () => {
    state.isAdmin = false;

    render(<AiAssistantBody />);

    expect(
      screen.queryByRole("button", { name: "Delete How we read metrics" })
    ).toBeNull();
    expect(screen.getByText("Admins write")).toBeInTheDocument();
  });
});
