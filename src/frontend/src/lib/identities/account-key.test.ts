/**
 * The `?acct=` key must survive a round trip for ANY account_id — including
 * one containing the separator itself — and a malformed or hand-mangled key
 * must read as "nothing selected", never as a different account.
 */
import { describe, expect, it } from "vitest";

import { accountKey, parseAccountKey } from "./account-key";

describe("account key", () => {
  it.each([
    ["plain", "dev-42"],
    ["contains the separator", "team:lead:42"],
    ["contains url metacharacters", "a b?&=#/%"],
  ])("round-trips an account_id that is %s", (_name, accountId) => {
    const ref = {
      source: "github",
      source_id: "01900000-0000-7000-8000-00000000aa01",
      account_id: accountId,
    };
    expect(parseAccountKey(accountKey(ref))).toEqual(ref);
  });

  it.each([
    ["undefined", undefined],
    ["empty", ""],
    ["two parts", "github:x"],
    ["four parts", "github:x:y:z"],
    ["an empty part", "github::y"],
    ["a truncated percent-escape", "github:x:dev%2"],
  ])("reads a key that is %s as no selection", (_name, key) => {
    expect(parseAccountKey(key)).toBeNull();
  });
});
