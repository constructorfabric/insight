import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { CustomPageShell } from "./custom-page-shell";

const SAVED = "insight.custom.assistant-hidden";

function shell() {
  return render(
    <CustomPageShell chat={<p>assistant</p>}>
      <p>dashboard</p>
    </CustomPageShell>
  );
}

beforeEach(() => {
  window.localStorage.clear();
});

describe("<CustomPageShell>", () => {
  it("keeps the assistant collapsed to a strip by default, with the thread mounted", () => {
    shell();

    expect(screen.getByText("assistant")).not.toBeVisible();
    expect(screen.getByText("assistant")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Open assistant" })
    ).toBeVisible();
    expect(screen.getByText("dashboard")).toBeVisible();
  });

  it("opens the assistant beside the content", async () => {
    shell();

    await userEvent.click(screen.getByRole("button", { name: "Open assistant" }));

    expect(screen.getByText("assistant")).toBeVisible();
    expect(screen.getByText("dashboard")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Open assistant" })).toBeNull();
  });

  it("collapses it back, and keeps it mounted so the thread survives", async () => {
    shell();

    await userEvent.click(screen.getByRole("button", { name: "Open assistant" }));
    await userEvent.click(
      screen.getByRole("button", { name: "Collapse assistant" })
    );

    expect(screen.getByText("assistant")).not.toBeVisible();
    expect(screen.getByText("assistant")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Open assistant" })
    ).toBeVisible();
  });

  it("remembers that it was opened", async () => {
    const { unmount } = shell();
    await userEvent.click(screen.getByRole("button", { name: "Open assistant" }));
    unmount();

    shell();

    expect(screen.getByText("assistant")).toBeVisible();
  });

  it.each([
    ["open", "0", true],
    ["collapsed", "1", false],
  ])("starts %s when that was the saved choice", (_choice, saved, visible) => {
    window.localStorage.setItem(SAVED, saved);

    shell();

    if (visible) expect(screen.getByText("assistant")).toBeVisible();
    else expect(screen.getByText("assistant")).not.toBeVisible();
  });
});
