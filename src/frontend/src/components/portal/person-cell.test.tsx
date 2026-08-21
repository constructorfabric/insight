// @vitest-environment jsdom
/**
 * One person must read the same everywhere: name by the card's precedence
 * (display name, then username, then email, then the id), an identifying second
 * line that never repeats the name, and a leaver marked so nobody merges into
 * them.
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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
    ["the handle over the address", { email: "a@example.com", username: "alee" }, "alee"],
    ["the address when there is no handle", { email: "a@example.com" }, "a@example.com"],
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

  // A person automation minted may carry no attributes at all; printing its id
  // as a name would state the id twice and imply the journal knows something it
  // does not.
  it("reads an attribute-less person as unnamed rather than naming it by its id", () => {
    render(<PersonCell person={person({})} />);

    expect(screen.getByText(/unnamed person/i)).toBeInTheDocument();
    expect(
      screen.getAllByText("01900000-0000-7000-8000-000000000001"),
    ).toHaveLength(1);
  });

  // The picker is where the wrong person gets chosen, and a stub automation
  // minted is the wrong side of a merge — its counterpart holds the history.
  // The badge must not name one origin: a sign-in and a roster listing both
  // produce such a person, and the wording is the same warning either way.
  it("marks a person the journal knows only from an automatic mint", async () => {
    render(
      <PersonCell person={person({ display_name: "New Joiner", provisional: true })} />,
    );

    // One word, so the badge cannot widen the row it sits in — and the warning
    // it stands for is reachable, not hover-only: this mark is what says a
    // person is the wrong side of a merge.
    const badge = screen.getByText(/^provisional$/i);
    expect(badge).toBeInTheDocument();
    expect(badge).toHaveAttribute("tabindex", "0");

    await userEvent.hover(badge);
    expect(
      await screen.findByText(/created by automation, not confirmed/i),
    ).toBeInTheDocument();
    // A roster mint is not a sign-in. Naming one origin tells an operator the
    // wrong story about half of these people.
    expect(screen.queryByText(/first sign-in/i)).not.toBeInTheDocument();
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
