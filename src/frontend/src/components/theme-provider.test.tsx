import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { ThemeProvider, useTheme, type Theme } from "@/components/theme-provider";

type MediaListener = (e: { matches: boolean }) => void;

let systemPrefersDark = false;
let mediaListeners: MediaListener[] = [];

function stubMatchMedia() {
  window.matchMedia = ((query: string) =>
    ({
      matches: systemPrefersDark,
      media: query,
      onchange: null,
      addEventListener: (_type: string, cb: MediaListener) => {
        mediaListeners.push(cb);
      },
      removeEventListener: (_type: string, cb: MediaListener) => {
        mediaListeners = mediaListeners.filter((l) => l !== cb);
      },
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }) as unknown as MediaQueryList) as typeof window.matchMedia;
}

function Probe() {
  const { theme, setTheme } = useTheme();
  return (
    <div>
      <span data-testid="theme">{theme}</span>
      {(["light", "dark", "system"] as Theme[]).map((t) => (
        <button key={t} onClick={() => setTheme(t)}>
          set-{t}
        </button>
      ))}
    </div>
  );
}

describe("ThemeProvider", () => {
  beforeEach(() => {
    systemPrefersDark = false;
    mediaListeners = [];
    stubMatchMedia();
    localStorage.clear();
    document.documentElement.classList.remove("light", "dark");
  });

  afterEach(() => {
    document.documentElement.classList.remove("light", "dark");
  });

  it("applies an explicit theme to the document root", () => {
    render(
      <ThemeProvider defaultTheme="dark">
        <Probe />
      </ThemeProvider>,
    );

    expect(screen.getByTestId("theme")).toHaveTextContent("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("resolves the system theme from the media query and follows changes", () => {
    systemPrefersDark = true;
    render(
      <ThemeProvider defaultTheme="system">
        <Probe />
      </ThemeProvider>,
    );

    expect(document.documentElement.classList.contains("dark")).toBe(true);

    act(() => {
      for (const cb of mediaListeners) cb({ matches: false });
    });
    expect(document.documentElement.classList.contains("light")).toBe(true);
  });

  it("persists a chosen theme to localStorage and reads it back on mount", async () => {
    const { unmount } = render(
      <ThemeProvider storageKey="test-theme">
        <Probe />
      </ThemeProvider>,
    );

    act(() => {
      screen.getByRole("button", { name: "set-dark" }).click();
    });
    expect(localStorage.getItem("test-theme")).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    unmount();

    render(
      <ThemeProvider storageKey="test-theme">
        <Probe />
      </ThemeProvider>,
    );
    expect(screen.getByTestId("theme")).toHaveTextContent("dark");
  });
});
