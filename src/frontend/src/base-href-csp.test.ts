// The index.html <base>-injecting inline script must stay byte-identical to
// the sha256 the nginx CSP allows: a reformat breaks the hash, CSP blocks the
// script, and prefix serving (/exp/<name>/) silently dies.
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

const root = join(__dirname, "..");
const html = readFileSync(join(root, "index.html"), "utf8");
const scriptElement = /<script>([\s\S]*?)<\/script>/.exec(html);

describe("base-href bootstrap script", () => {
  it("exists as the first resource-affecting head element", () => {
    expect(scriptElement).toBeTruthy();
    expect(scriptElement?.index).toBeGreaterThan(html.indexOf("<head>"));
    expect(scriptElement?.index).toBeLessThan(html.indexOf("<link"));
  });

  it("hash matches the nginx CSP allowance", () => {
    const digest = createHash("sha256")
      .update(scriptElement?.[1] ?? "")
      .digest("base64");
    const nginx = readFileSync(
      join(root, "nginx/default.conf.template"),
      "utf8"
    );
    expect(nginx).toContain(`'sha256-${digest}'`);
  });

  it("prefix regex accepts experiment slugs and rejects lookalikes", () => {
    // Test the regex the script actually ships, not a copy of it.
    const source = /location\.pathname\.match\(\s*\/((?:\\.|[^/])*)\/\s*\)/.exec(
      scriptElement?.[1] ?? ""
    )?.[1];
    expect(source).toBeTruthy();
    const re = new RegExp(source ?? "(?!)");
    expect("/exp/demo/".match(re)?.[0]).toBe("/exp/demo");
    expect("/exp/widget-alpha/deep/route".match(re)?.[0]).toBe(
      "/exp/widget-alpha"
    );
    expect("/exp/a".match(re)?.[0]).toBe("/exp/a");
    for (const miss of ["/", "/expunge/x", "/exp/", "/exp/-bad", "/EXP/demo"]) {
      expect(miss.match(re), `should reject: ${miss}`).toBeNull();
    }
  });
});
