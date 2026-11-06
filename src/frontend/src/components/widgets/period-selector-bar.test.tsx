/**
 * Component coverage for `<PeriodSelectorBar>`: period tab switching, the
 * custom-range trigger label, and the calendar popover's Apply / Cancel /
 * Clear actions. A fixed custom range keeps date labels deterministic.
 */

import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { CustomRange } from "@/types/insight";

import { PeriodSelectorBar } from "./period-selector-bar";

const CUSTOM: CustomRange = { from: "2026-07-01", to: "2026-07-10" };

function renderBar(customRange: CustomRange | null = null) {
  const onPeriodChange = vi.fn();
  const onRangeChange = vi.fn();
  render(
    <PeriodSelectorBar
      period="month"
      customRange={customRange}
      onPeriodChange={onPeriodChange}
      onRangeChange={onRangeChange}
    />,
  );
  return { onPeriodChange, onRangeChange };
}

describe("<PeriodSelectorBar>", () => {
  it("renders the four period tabs", () => {
    renderBar();
    for (const label of ["Week", "Month", "Quarter", "Year"]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
  });

  it("fires onPeriodChange when a period tab is clicked", async () => {
    const { onPeriodChange } = renderBar();
    await userEvent.click(screen.getByText("Week"));
    expect(onPeriodChange).toHaveBeenCalledWith("week");
  });

  it("shows the active custom range on the calendar trigger", () => {
    renderBar(CUSTOM);
    expect(screen.getByText("1 Jul – 10 Jul")).toBeInTheDocument();
    expect(screen.getByText("UTC")).toBeInTheDocument();
  });

  it("opens the calendar popover and cancels without applying", { timeout: 30_000 }, async () => {
    const { onRangeChange } = renderBar(CUSTOM);
    await userEvent.click(screen.getByText("1 Jul – 10 Jul"));
    // Header echoes the pre-seeded range in long form.
    expect(
      await screen.findByText(/1 Jul 2026\s*–\s*10 Jul 2026/),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => {
      expect(
        screen.queryByRole("button", { name: "Apply" }),
      ).not.toBeInTheDocument();
    });
    expect(onRangeChange).not.toHaveBeenCalled();
  });

  it("applies the pre-seeded range through the Apply button", { timeout: 30_000 }, async () => {
    const { onRangeChange } = renderBar(CUSTOM);
    await userEvent.click(screen.getByText("1 Jul – 10 Jul"));
    const apply = await screen.findByRole("button", { name: "Apply" });
    expect(apply).toBeEnabled();
    await userEvent.click(apply);
    expect(onRangeChange).toHaveBeenCalledWith(CUSTOM);
  });

  it("clears an active custom range", { timeout: 30_000 }, async () => {
    const { onRangeChange } = renderBar(CUSTOM);
    await userEvent.click(screen.getByText("1 Jul – 10 Jul"));
    await userEvent.click(
      await screen.findByRole("button", { name: "Clear" }),
    );
    expect(onRangeChange).toHaveBeenCalledWith(null);
  });

  it("hides Clear when no custom range is active", { timeout: 30_000 }, async () => {
    renderBar(null);
    // Without a custom range the trigger carries the resolved period range.
    await userEvent.click(screen.getByText("UTC").closest("button")!);
    await screen.findByRole("button", { name: "Apply" });
    expect(
      screen.queryByRole("button", { name: "Clear" }),
    ).not.toBeInTheDocument();
  });
});

describe("the calendar is usable from the keyboard", () => {
  it("moves focus to the next day on ArrowRight and picks it on Enter", { timeout: 30_000 }, async () => {
    renderBar(CUSTOM);
    await userEvent.click(screen.getByText("1 Jul – 10 Jul"));
    const fifth = await screen.findByRole("button", { name: /July 5th, 2026/ });

    act(() => fifth.focus());
    await userEvent.keyboard("{ArrowRight}");

    const sixth = screen.getByRole("button", { name: /July 6th, 2026/ });
    expect(document.activeElement).toBe(sixth);

    await userEvent.keyboard("{Enter}");

    const header = await screen.findByText(/^6 Jul 2026/);
    expect(header).toHaveTextContent("6 Jul 2026 → pick end date");
  });
});

describe("arrow keys inside the calendar stay inside it", () => {
  it.each(["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"])(
    "%s on the previous-month button keeps focus in the popover",
    { timeout: 30_000 },
    async (key) => {
      renderBar(CUSTOM);
      await userEvent.click(screen.getByText("1 Jul – 10 Jul"));
      const previous = await screen.findByRole("button", { name: /previous month/i });

      act(() => previous.focus());
      await userEvent.keyboard(`{${key}}`);

      expect(document.activeElement, key).toBe(previous);
    },
  );
});

describe("the calendar offers no day that has not happened yet", () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(2026, 6, 15, 10, 30)); // local 2026-07-15
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it.each([
    { when: "yesterday", day: /July 14th, 2026/, enabled: true },
    { when: "today", day: /July 15th, 2026/, enabled: true },
    { when: "tomorrow", day: /July 16th, 2026/, enabled: false },
    { when: "next month", day: /August 1st, 2026/, enabled: false },
  ])("a day $when is pickable: $enabled", { timeout: 30_000 }, async ({ day, enabled }) => {
    renderBar(CUSTOM);
    await userEvent.click(screen.getByText("1 Jul – 10 Jul"));

    const button = await screen.findByRole("button", { name: day });

    if (enabled) expect(button, String(day)).toBeEnabled();
    else expect(button, String(day)).toBeDisabled();
  });
});

describe("west of UTC in the evening, the calendar offers the UTC day the page reports on", () => {
  const suiteZone = process.env.TZ;

  beforeEach(() => {
    process.env.TZ = "America/New_York";
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-06-16T00:00:00Z")); // local 2026-06-15 20:00
  });

  afterEach(() => {
    vi.useRealTimers();
    process.env.TZ = suiteZone;
  });

  async function openPickerOnRangeEndingOnTheUtcDay() {
    render(
      <PeriodSelectorBar
        period="week"
        customRange={null}
        activeRange={{ from: "2026-06-10", to: "2026-06-16" }}
        onPeriodChange={vi.fn()}
        onRangeChange={vi.fn()}
      />,
    );
    await userEvent.click(screen.getByLabelText(/Custom date range/));
  }

  it.each([
    { when: "the local day", day: /June 15th, 2026/, enabled: true },
    { when: "the UTC day", day: /June 16th, 2026/, enabled: true },
    { when: "the day after the UTC day", day: /June 17th, 2026/, enabled: false },
  ])("$when is pickable: $enabled", { timeout: 30_000 }, async ({ day, enabled }) => {
    await openPickerOnRangeEndingOnTheUtcDay();

    const button = await screen.findByRole("button", { name: day });

    if (enabled) expect(button, String(day)).toBeEnabled();
    else expect(button, String(day)).toBeDisabled();
  });

  it("lets the seeded range ending on the UTC day be applied", { timeout: 30_000 }, async () => {
    await openPickerOnRangeEndingOnTheUtcDay();

    expect(await screen.findByRole("button", { name: "Apply" })).toBeEnabled();
  });
});

describe("the range label names the window the page reports on", () => {
  it.each([
    { under: "a preset", customRange: null, activeRange: { from: "2026-07-02", to: "2026-07-08" } },
    { under: "a custom range", customRange: CUSTOM, activeRange: { from: "2026-07-01", to: "2026-07-10" } },
  ])("prints the range the page hands it under $under", ({ customRange, activeRange }) => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(2026, 10, 5, 10, 30));
    try {
      render(
        <PeriodSelectorBar
          period="week"
          customRange={customRange}
          activeRange={activeRange}
          onPeriodChange={vi.fn()}
          onRangeChange={vi.fn()}
        />,
      );
      const day = (iso: string) => String(Number(iso.slice(8)));
      const label = `${day(activeRange.from)} Jul – ${day(activeRange.to)} Jul`;

      expect(screen.getByText(label)).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: `Custom date range: ${label}` }),
      ).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("the range label says which year", () => {
  it("keeps the year out of the way when both ends are this year", () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(2026, 10, 5, 10, 30));
    try {
      renderBar({ from: "2026-07-01", to: "2026-07-10" });
      expect(screen.getByText("1 Jul – 10 Jul")).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("names the year when the range crosses one", () => {
    // "31 Jul – 30 Jul" for a whole year read as a typo, or as a week: the
    // reader had to already know the answer to decode the label.
    renderBar({ from: "2025-07-31", to: "2026-07-30" });
    expect(screen.getByText("31 Jul 2025 – 30 Jul 2026")).toBeInTheDocument();
  });

  it("names the year when the range is not in this one", () => {
    renderBar({ from: "2024-03-01", to: "2024-03-31" });
    expect(screen.getByText("1 Mar 2024 – 31 Mar 2024")).toBeInTheDocument();
  });
});
