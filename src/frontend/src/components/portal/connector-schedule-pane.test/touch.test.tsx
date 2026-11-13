import { mocks, pane, editCustom } from "./fixtures";
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";

function row(index: number) {
  return screen.getByRole("textbox", { name: `Cron expression ${index}` });
}

function message(input: HTMLElement) {
  return document.getElementById(input.getAttribute("aria-describedby")!)!;
}

async function addRow() {
  await userEvent.click(
    screen.getByRole("button", { name: "+ Add expression" })
  );
}

it("focuses a new empty row with an example and a quiet reason Save is disabled", async () => {
  render(pane());
  await editCustom();
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  await addRow();
  const input = row(2);
  expect(input).toHaveFocus();
  expect(input).toHaveValue("");
  expect(input).toHaveAttribute("placeholder", "0 */6 * * *");
  expect(input).toHaveAttribute("aria-invalid", "false");
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(
    screen.queryByText("Enter a valid schedule before saving.")
  ).not.toBeInTheDocument();
  expect(message(input)).toHaveTextContent(
    "Enter an expression or remove the row"
  );
  expect(message(input)).toHaveClass("text-muted-foreground");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(mocks.save).not.toHaveBeenCalled();
});

it("keeps an untouched empty row quiet after blur and lets its removal enable Save", async () => {
  render(pane());
  await editCustom();
  await addRow();
  const input = row(2);
  await userEvent.tab();
  expect(input).not.toHaveFocus();
  expect(input).toHaveAttribute("aria-invalid", "false");
  expect(message(input)).not.toHaveAttribute("role", "alert");
  expect(message(input)).toHaveTextContent(
    "Enter an expression or remove the row"
  );
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  await userEvent.click(
    screen.getByRole("button", { name: "Remove expression 2" })
  );
  expect(
    screen.queryByRole("textbox", { name: "Cron expression 2" })
  ).not.toBeInTheDocument();
  expect(row(1)).toHaveFocus();
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
});

it("debounces invalid typing from the last edit and updates revealed errors live", async () => {
  render(pane());
  await editCustom();
  await addRow();
  const input = row(2);
  vi.useFakeTimers({ toFake: ["Date", "setTimeout", "clearTimeout"] });
  fireEvent.change(input, { target: { value: "0 25 * * *" } });
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  act(() => vi.advanceTimersByTime(499));
  expect(input).toHaveAttribute("aria-invalid", "false");
  fireEvent.change(input, { target: { value: "0 26 * * *" } });
  act(() => vi.advanceTimersByTime(499));
  expect(input).toHaveAttribute("aria-invalid", "false");
  act(() => vi.advanceTimersByTime(1));
  expect(input).toHaveFocus();
  expect(input).toHaveAttribute("aria-invalid", "true");
  expect(message(input)).toHaveAttribute("role", "alert");
  const error = message(input).textContent;
  fireEvent.change(input, { target: { value: "0 6 * *" } });
  expect(message(input).textContent).not.toBe(error);
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  fireEvent.change(input, { target: { value: "0 8 * * *" } });
  expect(input).toHaveAttribute("aria-invalid", "false");
  expect(message(input)).toBeEmptyDOMElement();
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  fireEvent.change(input, { target: { value: "0 25 * * *" } });
  expect(input).toHaveAttribute("aria-invalid", "true");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});

it("reveals every current row on a Save attempt while keeping later new rows quiet", async () => {
  render(pane());
  await editCustom();
  await addRow();
  fireEvent.submit(row(1).closest("form")!);
  expect(row(2)).toHaveAttribute("aria-invalid", "true");
  expect(message(row(2))).toHaveAttribute("role", "alert");
  expect(mocks.save).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  fireEvent.change(row(2), { target: { value: "0 8 * * *" } });
  expect(row(2)).toHaveAttribute("aria-invalid", "false");
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  await addRow();
  expect(row(3)).toHaveAttribute("aria-invalid", "false");
  expect(message(row(3))).toHaveTextContent(
    "Enter an expression or remove the row"
  );
  fireEvent.change(row(2), { target: { value: "0 25 * * *" } });
  fireEvent.submit(row(1).closest("form")!);
  expect(row(2)).toHaveAttribute("aria-invalid", "true");
  expect(row(3)).toHaveAttribute("aria-invalid", "true");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  expect(mocks.save).not.toHaveBeenCalled();
});

it("keeps revealed errors with their rows when an earlier row is removed", async () => {
  render(pane());
  await editCustom();
  await addRow();
  await addRow();
  fireEvent.change(row(3), { target: { value: "0 25 * * *" } });
  fireEvent.blur(row(3));
  expect(row(3)).toHaveAttribute("aria-invalid", "true");
  await userEvent.click(
    screen.getByRole("button", { name: "Remove expression 1" })
  );
  expect(row(1)).toHaveValue("");
  expect(row(1)).toHaveAttribute("aria-invalid", "false");
  expect(message(row(1))).toHaveTextContent(
    "Enter an expression or remove the row"
  );
  expect(row(2)).toHaveValue("0 25 * * *");
  expect(row(2)).toHaveAttribute("aria-invalid", "true");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});
