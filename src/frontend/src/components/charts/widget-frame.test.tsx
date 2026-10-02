import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

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
