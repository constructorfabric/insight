// @vitest-environment jsdom
/**
 * The one line on a person's page that says what the page cannot see.
 *
 * Its whole job is to keep two claims apart, so the tests are about which
 * sentence each blank section gets — not about layout.
 */
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { PersonCoverage } from "./person-coverage";

describe("PersonCoverage", () => {
  it("says nothing when every section reads", () => {
    const { container } = render(
      <PersonCoverage unmeasured={[]} inactive={[]} />
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("blames the instrument only where the pool is empty too", () => {
    render(<PersonCoverage unmeasured={["Task delivery"]} inactive={[]} />);
    expect(
      screen.getByText(/No data source is connected for Task delivery/)
    ).toBeTruthy();
  });

  it("blames nobody where the measurement works and the person is absent", () => {
    // The failure this guards: telling a reader their tracker is broken while
    // their colleagues' numbers for that same section sit on the next screen.
    render(
      <PersonCoverage unmeasured={[]} inactive={["Git output", "Wiki"]} />
    );
    expect(
      screen.getByText(/Nothing recorded in Git output and Wiki this period/)
    ).toBeTruthy();
    expect(screen.queryByText(/reaches us/)).toBeNull();
  });

  it("carries both reasons at once, each with its own sentence", () => {
    render(
      <PersonCoverage unmeasured={["Task delivery"]} inactive={["Wiki"]} />
    );
    expect(
      screen.getByText(/No data source is connected for Task delivery/)
    ).toBeTruthy();
    expect(screen.getByText(/Nothing recorded in Wiki/)).toBeTruthy();
  });
});
