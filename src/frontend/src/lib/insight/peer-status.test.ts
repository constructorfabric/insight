import { describe, expect, it } from "vitest";

import { peerStatusToStatus } from "./peer-status";

describe("peerStatusToStatus — maps rank to display status", () => {
  it("top → good, bottom → bad, in_pack and neutral stay calm", () => {
    expect(peerStatusToStatus("top")).toBe("good");
    expect(peerStatusToStatus("bottom")).toBe("bad");
    expect(peerStatusToStatus("in_pack")).toBe("neutral");
    expect(peerStatusToStatus("neutral")).toBe("neutral");
  });
});
