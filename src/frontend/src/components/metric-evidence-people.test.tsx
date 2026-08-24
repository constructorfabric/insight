import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { EvidencePersonRow } from "@/components/metric-evidence-context";
import { MetricEvidencePeople } from "@/components/metric-evidence-people";

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        index,
        start: index * 44,
      })),
    getTotalSize: () => count * 44,
    measureElement: vi.fn(),
  }),
}));

const drillable: EvidencePersonRow = {
  entityId: "e-ada",
  personId: "p-ada",
  name: "Ada Lovelace",
  value: 142,
  valueText: "142",
  target: {
    selection: {
      metric_key: "git.commits",
      entity: { type: "person", id: "e-ada" },
      period: { from: "2026-07-01", to: "2026-07-31" },
      filters: [],
      display_dimensions: [],
    },
    label: "Commits · Ada Lovelace",
  },
};

const unreadable: EvidencePersonRow = {
  entityId: "e-grace",
  personId: null,
  name: "Grace Hopper",
  value: 131,
  valueText: "131",
  target: null,
};

function draw(rows: EvidencePersonRow[] = [drillable, unreadable]) {
  const onDrill = vi.fn();
  render(
    <MetricEvidencePeople rows={rows} valueLabel="Commits" onDrill={onDrill} />
  );
  return { onDrill };
}

describe("MetricEvidencePeople", () => {
  it("maps rows and cells for a screen reader, despite the flex layout", () => {
    draw();

    // Two people plus the header row they sit under.
    expect(screen.getByRole("table")).toHaveAttribute("aria-rowcount", "3");
    expect(
      screen.getByRole("columnheader", { name: "Person" })
    ).toBeInTheDocument();
    expect(
      screen.getByRole("columnheader", { name: "Commits" })
    ).toBeInTheDocument();
    // Two people plus the header row.
    expect(screen.getAllByRole("row")).toHaveLength(3);
    expect(screen.getAllByRole("cell", { name: "142" })).toHaveLength(1);
  });

  it("says what the opener opens, rather than repeating the name", () => {
    draw();

    expect(
      screen.getByRole("button", { name: "Open records for Ada Lovelace" })
    ).toBeInTheDocument();
  });

  it("opens a person from the row and from its opener, once each", async () => {
    const user = userEvent.setup();
    const { onDrill } = draw();

    await user.click(screen.getByRole("cell", { name: "Ada Lovelace" }));
    expect(onDrill).toHaveBeenCalledTimes(1);

    // The opener must not also trigger the row it sits in.
    await user.click(
      screen.getByRole("button", { name: "Open records for Ada Lovelace" })
    );
    expect(onDrill).toHaveBeenCalledTimes(2);
    expect(onDrill).toHaveBeenLastCalledWith(drillable);
  });

  it("leaves a person with no readable records unopenable", async () => {
    const user = userEvent.setup();
    const { onDrill } = draw();

    expect(
      screen.queryByRole("button", { name: /Grace Hopper/ })
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("cell", { name: "Grace Hopper" }));

    expect(onDrill).not.toHaveBeenCalled();
  });
});
