import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { CustomPageShell } from "./custom-page-shell";

beforeEach(() => {
  window.localStorage.clear();
});

describe("<CustomPageShell>", () => {
  it("shows the assistant beside the content by default", () => {
    render(
      <CustomPageShell chat={<p>assistant</p>}>
        <p>dashboard</p>
      </CustomPageShell>
    );

    expect(screen.getByText("assistant")).toBeVisible();
    expect(screen.getByText("dashboard")).toBeVisible();
  });

  it("puts the assistant away, and keeps it mounted so the thread survives", async () => {
    render(
      <CustomPageShell chat={<p>assistant</p>}>
        <p>dashboard</p>
      </CustomPageShell>
    );

    await userEvent.click(
      screen.getByRole("button", { name: "Hide the assistant" })
    );

    expect(screen.getByText("assistant")).not.toBeVisible();
    expect(screen.getByText("assistant")).toBeInTheDocument();
    expect(screen.getByText("dashboard")).toBeVisible();
  });

  it("brings it back", async () => {
    render(
      <CustomPageShell chat={<p>assistant</p>}>
        <p>dashboard</p>
      </CustomPageShell>
    );

    await userEvent.click(
      screen.getByRole("button", { name: "Hide the assistant" })
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Show the assistant" })
    );

    expect(screen.getByText("assistant")).toBeVisible();
  });

  it("remembers that it was put away", async () => {
    const { unmount } = render(
      <CustomPageShell chat={<p>assistant</p>}>
        <p>dashboard</p>
      </CustomPageShell>
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Hide the assistant" })
    );
    unmount();

    render(
      <CustomPageShell chat={<p>assistant</p>}>
        <p>dashboard</p>
      </CustomPageShell>
    );

    expect(screen.getByText("assistant")).not.toBeVisible();
  });
});
