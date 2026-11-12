import { mocks, pane, read } from "./fixtures";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";

async function hourly() {
  mocks.query.data!.schedule.requested!.crons = ["22 * * * *"];
  mocks.save.mockResolvedValue(read());
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  const input = screen.getByRole<HTMLInputElement>("spinbutton", {
    name: "Minute",
  });
  await userEvent.click(input);
  return input;
}

async function typeAtEnd(input: HTMLInputElement, digits: string) {
  for (const digit of digits) {
    input.setSelectionRange(input.value.length, input.value.length);
    await userEvent.keyboard(digit);
  }
}

it("replaces a full minute and saves the new cron even when selection collapses", async () => {
  const input = await hourly();
  expect(input).toHaveValue("22");
  await typeAtEnd(input, "3");
  expect(input).toHaveValue("3");
  await typeAtEnd(input, "0");
  expect(input).toHaveValue("30");
  expect(screen.getByText("30 * * * *")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(mocks.save).toHaveBeenCalledWith({
    etag: '"2"',
    crons: ["30 * * * *"],
  });
});

it("starts a new minute on the third digit with a collapsed caret", async () => {
  const input = await hourly();
  await typeAtEnd(input, "304");
  expect(input).toHaveValue("4");
  expect(input).toHaveAttribute("aria-valuenow", "4");
  await userEvent.tab();
  expect(input).toHaveValue("04");
});

it.each(["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End"])(
  "replaces the minute after %s even when the browser loses the selection",
  async (key) => {
    const input = await hourly();
    await userEvent.keyboard(`{${key}}`);
    await typeAtEnd(input, "3");
    expect(input).toHaveValue("3");
    await typeAtEnd(input, "0");
    expect(input).toHaveValue("30");
  }
);

it("restarts replacement on each pointer interaction without relying on selection", async () => {
  const input = await hourly();
  await typeAtEnd(input, "3");
  fireEvent.pointerDown(input);
  await typeAtEnd(input, "4");
  expect(input).toHaveValue("4");
  fireEvent.click(input);
  await typeAtEnd(input, "5");
  expect(input).toHaveValue("5");
});

it("accepts non-keyboard digit entry before the browser enforces maxlength", async () => {
  const input = await hourly();
  for (const digit of "30") {
    input.setSelectionRange(input.value.length, input.value.length);
    const event = new InputEvent("beforeinput", {
      bubbles: true,
      cancelable: true,
      inputType: "insertText",
      data: digit,
    });
    fireEvent(input, event);
    expect(event.defaultPrevented).toBe(true);
  }
  expect(input).toHaveValue("30");
  expect(screen.getByText("30 * * * *")).toBeInTheDocument();
});

it("keeps invalid digit segments visible, disables Save, then replaces them", async () => {
  const input = await hourly();
  await typeAtEnd(input, "75");
  expect(input).toHaveValue("75");
  expect(input).toHaveAttribute("aria-invalid", "true");
  expect(screen.getByText("Enter a minute from 0 to 59.")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  await typeAtEnd(input, "4");
  expect(input).toHaveValue("4");
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  await userEvent.keyboard("{Backspace}");
  expect(input).toHaveValue("");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});

it.each(["30", ":30", "75"])(
  "preserves pasted minute %s and its validation",
  async (text) => {
    const input = await hourly();
    await userEvent.paste(text);
    expect(input).toHaveValue(text.replace(/^:/, ""));
    expect(input).toHaveAttribute("aria-invalid", String(text === "75"));
    const save = screen.getByRole("button", { name: "Save" });
    if (text === "75") expect(save).toBeDisabled();
    else expect(save).toBeEnabled();
  }
);
