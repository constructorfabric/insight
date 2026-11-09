import { describe, expect, it } from "vitest";

import { matchesAccountSearch } from "./account-search";

const ACCOUNT = {
  source: "gitlab-commit-email",
  account_id: "gl-1042",
  email: "Ann.Lee@Example.com",
  username: "annlee",
  display_name: "Ann Lee",
};

describe("matchesAccountSearch", () => {
  it.each([
    ["a blank needle keeps everything", "", ACCOUNT, true],
    ["a whitespace needle keeps everything", "   ", ACCOUNT, true],
    ["email, case-insensitively", "ANN.LEE@example", ACCOUNT, true],
    ["email, by a substring", "lee@exa", ACCOUNT, true],
    ["username", "nnle", ACCOUNT, true],
    ["account id", "1042", ACCOUNT, true],
    ["display name", "ann l", ACCOUNT, true],
    ["a needle in none of the fields", "zzz", ACCOUNT, false],
    [
      "a missing field does not match a blank-looking needle",
      "x",
      {
        source: "hr",
        account_id: "1",
        email: null,
        username: null,
        display_name: null,
      },
      false,
    ],
    ["a source at its first segment", "gitlab", ACCOUNT, true],
    ["a source at a later segment", "commit", ACCOUNT, true],
    ["a source at the last segment", "email", ACCOUNT, true],
    [
      "a source across a boundary, with either separator",
      "gitlab_commit-email",
      ACCOUNT,
      true,
    ],
    [
      "hub does not match github",
      "hub",
      {
        ...ACCOUNT,
        source: "github",
        email: null,
        username: null,
        display_name: null,
        account_id: "1",
      },
      false,
    ],
    [
      "entra matches ms_entra",
      "entra",
      {
        ...ACCOUNT,
        source: "ms_entra",
        email: null,
        username: null,
        display_name: null,
        account_id: "1",
      },
      true,
    ],
    [
      "a source segment match is case-insensitive",
      "GitLab",
      {
        ...ACCOUNT,
        email: null,
        username: null,
        display_name: null,
        account_id: "1",
      },
      true,
    ],
    [
      "a needle that starts mid-segment of a source does not match",
      "itlab",
      {
        ...ACCOUNT,
        email: null,
        username: null,
        display_name: null,
        account_id: "1",
      },
      false,
    ],
  ])("%s", (_case, needle, account, expected) => {
    expect(matchesAccountSearch(account, needle)).toBe(expected);
  });
});
