import { readFileSync } from "node:fs";
import { join } from "node:path";

import { expect, it } from "vitest";
import { validateScheduleCrons } from "./connector-schedule-validation";

type ValidationCase = {
  name: string;
  crons: string[];
  valid: boolean;
};

const cases = JSON.parse(
  readFileSync(
    join(
      __dirname,
      "../../../../../tests/fixtures/connector-schedule-validation.json"
    ),
    "utf8"
  )
) as ValidationCase[];

it.each(cases)(
  "matches service validation: $name ($crons)",
  ({ crons, valid }) => {
    expect(validateScheduleCrons(crons).length === 0).toBe(valid);
  }
);
