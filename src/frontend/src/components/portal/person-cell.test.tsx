// @vitest-environment jsdom
/**
 * One person must read the same everywhere: name by the card's precedence
 * (display name → email → username → id), an identifying second line that
 * never repeats the name, and a leaver marked so nobody merges into them.
 */
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import "@/i18n";
import type { PersonSummary } from "@/api/identity-client";

import { personDisplayName } from "@/lib/identities/person-display";

import { PersonCell } from "./person-cell";

function person(over: Partial<PersonSummary>): PersonSummary {
  return { person_id: "01900000-0000-7000-8000-000000000001", ...over };
}

describe("personDisplayName", () => {
  it.each<[string, Partial<PersonSummary>, string]>([
    ["display name first", { display_name: "Ann Lee", email: "a@example.com" }, "Ann Lee"],
    ["email when unnamed", { email: "a@example.com", username: "alee" }, "a@example.com"],
    ["username for a git-only identity", { username: "alee" }, "alee"],
    ["the id when nothing else exists", {}, "01900000-0000-7000-8000-000000000001"],
  ])("picks %s", (_name, fields, expected) => {
    expect(personDisplayName(person(fields))).toBe(expected);
  });
});

describe("PersonCell", () => {
  it("identifies without repeating: the name field is skipped in the detail line", () => {
    render(
      <PersonCell
        person={person({ email: "a@example.com", job_title: "Engineer" })}
      />,
    );

    // email became the name — the detail line must not repeat it.
    expect(screen.getByText("a@example.com")).toBeInTheDocument();
    expect(screen.getByText("Engineer")).toBeInTheDocument();
    expect(screen.queryAllByText(/a@example\.com/)).toHaveLength(1);
  });

  it("marks a terminated person", () => {
    render(
      <PersonCell
        person={person({ display_name: "Gone Person", status: "terminated" })}
      />,
    );

    expect(screen.getByText(/terminated/i)).toBeInTheDocument();
  });

  it("does not mark an active person", () => {
    render(
      <PersonCell person={person({ display_name: "Here Person", status: "active" })} />,
    );

    expect(screen.queryByText(/terminated/i)).not.toBeInTheDocument();
  });
});
