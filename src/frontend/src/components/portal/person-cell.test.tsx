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

  // Two records of one human is the normal shape of a conflict, so name and
  // address are exactly the fields that fail to tell them apart.
  it("always shows the person id, and offers it for copying", () => {
    render(<PersonCell person={person({ display_name: "Ann Lee" })} />);

    expect(
      screen.getByText("01900000-0000-7000-8000-000000000001"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", {
        name: /copy 01900000-0000-7000-8000-000000000001/i,
      }),
    ).toBeInTheDocument();
  });

  // A person minted at first sign-in carries no attributes until the resolver
  // attaches the roster's; printing its id as a name would state the id twice
  // and imply the journal knows something it does not.
  it("reads an attribute-less person as unnamed rather than naming it by its id", () => {
    render(<PersonCell person={person({})} />);

    expect(screen.getByText(/unnamed person/i)).toBeInTheDocument();
    expect(
      screen.getAllByText("01900000-0000-7000-8000-000000000001"),
    ).toHaveLength(1);
  });

  // The picker is where the wrong person gets chosen, and a stub minted at a
  // sign-in is the wrong side of a merge — its counterpart holds the history.
  it("marks a person the journal knows only from a first sign-in", () => {
    render(
      <PersonCell person={person({ display_name: "New Joiner", provisional: true })} />,
    );

    expect(screen.getByText(/provisional/i)).toBeInTheDocument();
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
