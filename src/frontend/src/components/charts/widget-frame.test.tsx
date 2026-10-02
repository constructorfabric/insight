import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { WidgetFrame } from "./widget-frame";

describe("<WidgetFrame>", () => {
  it("shows the title, the subtitle and the action beside them", () => {
    render(
      <WidgetFrame
        title="Traffic"
        subtitle="Visits per day"
        action={<button type="button">Open</button>}
        state="ready"
      >
        <p>chart</p>
      </WidgetFrame>
    );

    expect(
      screen.getByRole("heading", { name: "Traffic" })
    ).toBeInTheDocument();
    expect(screen.getByText("Visits per day")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open" })).toBeInTheDocument();
    expect(screen.getByText("chart")).toBeInTheDocument();
  });

  it("holds its body back while loading", () => {
    render(
      <WidgetFrame title="Traffic" state="loading">
        <p>chart</p>
      </WidgetFrame>
    );

    expect(screen.getByRole("status")).toHaveTextContent("Loading");
    expect(screen.queryByText("chart")).not.toBeInTheDocument();
  });

  it("offers a retry when the data could not be read", () => {
    const retry = vi.fn();
    render(
      <WidgetFrame
        title="Traffic"
        state="error"
        errorLabel="The metric failed."
        onRetry={retry}
      >
        <p>chart</p>
      </WidgetFrame>
    );

    fireEvent.click(screen.getByRole("button", { name: "Retry" }));

    expect(
      screen.getByRole("status", { name: "The metric failed." })
    ).toBeInTheDocument();
    expect(retry).toHaveBeenCalledOnce();
  });

  it.each(["Enter", " "])(
    "opens its body with %j as well as a click",
    (key) => {
      const open = vi.fn();
      render(
        <WidgetFrame
          title="Traffic"
          state="ready"
          onBodyActivate={open}
          bodyLabel="Show the data"
        >
          <p>chart</p>
        </WidgetFrame>
      );
      const body = screen.getByRole("button", { name: "Show the data" });

      fireEvent.keyDown(body, { key });
      fireEvent.click(body);

      expect(open).toHaveBeenCalledTimes(2);
    }
  );

  it("lets a long body scroll inside the frame when asked", () => {
    render(
      <WidgetFrame title="Rows" state="ready" tall>
        <p>table</p>
      </WidgetFrame>
    );

    const body = screen.getByText("table").parentElement;

    expect(body).toHaveClass("overflow-auto");
    expect(body).toHaveClass("pt-0");
  });

  it("leaves its body inert when nothing opens from it", () => {
    render(
      <WidgetFrame title="Traffic" state="ready">
        <p>chart</p>
      </WidgetFrame>
    );

    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
  it.each([
    [false, "304px"],
    [true, "464px"],
  ] as const)("stands tall=%s at %s", (tall, height) => {
    const { container } = render(
      <WidgetFrame title="Rows" state="ready" tall={tall}>
        <p>table</p>
      </WidgetFrame>
    );

    expect(container.firstElementChild).toHaveStyle({ height });
  });
  it("lets anything taller than a chart, such as a long refusal, be scrolled to", () => {
    render(
      <WidgetFrame title="Traffic" state="ready">
        <p>chart</p>
      </WidgetFrame>
    );

    expect(screen.getByText("chart").parentElement).toHaveClass(
      "overflow-auto"
    );
  });
});

describe("<WidgetFrame> full screen", () => {
  afterEach(() => {
    Object.defineProperty(document, "fullscreenElement", {
      configurable: true,
      value: null,
    });
  });

  function enterFullscreen(element: Element) {
    Object.defineProperty(document, "fullscreenElement", {
      configurable: true,
      value: element,
    });
    act(() => {
      document.dispatchEvent(new Event("fullscreenchange"));
    });
  }

  it("offers no full screen button unless asked", () => {
    render(
      <WidgetFrame title="Rows" state="ready">
        <p>table</p>
      </WidgetFrame>
    );

    expect(
      screen.queryByRole("button", { name: "Full screen" })
    ).not.toBeInTheDocument();
  });

  it("puts the widget in full screen and takes it out again", () => {
    const request = vi.fn().mockResolvedValue(undefined);
    const exit = vi.fn().mockResolvedValue(undefined);
    HTMLElement.prototype.requestFullscreen = request;
    document.exitFullscreen = exit;
    render(
      <WidgetFrame title="Traffic" state="ready" fullscreen>
        <p>chart</p>
      </WidgetFrame>
    );

    fireEvent.click(screen.getByRole("button", { name: "Full screen" }));
    const shown = request.mock.contexts[0] as HTMLElement;
    enterFullscreen(shown);
    fireEvent.click(screen.getByRole("button", { name: "Exit full screen" }));

    expect(shown).toContainElement(screen.getByText("chart"));
    expect(exit).toHaveBeenCalledOnce();
  });

  it("leaves full screen before opening what the body opens", () => {
    const exit = vi.fn().mockResolvedValue(undefined);
    document.exitFullscreen = exit;
    const open = vi.fn();
    render(
      <WidgetFrame
        title="Traffic"
        state="ready"
        fullscreen
        onBodyActivate={open}
        bodyLabel="Show the data"
      >
        <p>chart</p>
      </WidgetFrame>
    );

    enterFullscreen(
      screen.getByText("chart").closest("[data-fullscreen]") as Element
    );
    fireEvent.click(screen.getByRole("button", { name: "Show the data" }));

    expect(exit).toHaveBeenCalledOnce();
    expect(open).toHaveBeenCalledOnce();
  });
});
