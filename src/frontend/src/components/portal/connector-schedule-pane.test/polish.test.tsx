import { pane } from "./fixtures";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";

it("opens the editor focused on kit Runs with every preset in its established order and no native selects", async () => {
  const { container } = render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  const runs = screen.getByRole("combobox", { name: "Runs" });
  expect(runs).toHaveFocus();
  expect(runs.tagName).toBe("BUTTON");
  expect(container.querySelector("select")).toBeNull();
  expect(runs).toHaveTextContent("Every day");
  await userEvent.click(runs);
  const options = within(await screen.findByRole("listbox")).getAllByRole(
    "option"
  );
  expect(options.map((option) => option.textContent)).toEqual([
    "Every hour",
    "Every 2 hours",
    "Every 3 hours",
    "Every 4 hours",
    "Every 6 hours",
    "Every 8 hours",
    "Every 12 hours",
    "Every day",
    "Every weekday (Mon–Fri)",
    "Custom cron (UTC)",
  ]);
  await userEvent.click(screen.getByRole("option", { name: "Every 2 hours" }));
  expect(runs).toHaveTextContent("Every 2 hours");
  expect(screen.getByRole("group", { name: "Start time" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
});

it("separates the cron preview from UTC with a word space and a visible layout gap", async () => {
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  const cron = screen.getByText("0 6 * * *");
  const caption = cron.parentElement!;
  expect(cron.nextElementSibling).toHaveTextContent(/^UTC$/);
  expect(caption).toHaveTextContent(/^0 6 \* \* \* UTC$/);
  expect(caption).toHaveClass("flex", "flex-wrap", "gap-x-1");
});
