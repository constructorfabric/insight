import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { ChatProse } from "./chat-prose";

describe("<ChatProse>", () => {
  it("emphasises what the model wrapped in asterisks", () => {
    render(
      <p>
        <ChatProse text="You have **events** and **raw_data**." />
      </p>
    );

    expect(screen.getByText("events").tagName).toBe("STRONG");
    expect(screen.getByText("raw_data").tagName).toBe("STRONG");
    // And the asterisks themselves are gone.
    expect(screen.queryByText(/\*\*/)).not.toBeInTheDocument();
  });

  it("leaves prose with no emphasis exactly as written", () => {
    const text = "events holds author, day, event and lines.";

    render(
      <p>
        <ChatProse text={text} />
      </p>
    );

    expect(screen.getByText(text)).toBeInTheDocument();
  });

  it("keeps a lone asterisk as typed", () => {
    render(
      <p>
        <ChatProse text="rows * columns" />
      </p>
    );

    expect(screen.getByText("rows * columns")).toBeInTheDocument();
  });
});
