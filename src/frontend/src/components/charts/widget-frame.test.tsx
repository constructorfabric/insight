import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
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

  it("opens what the body opens on a click anywhere in it", () => {
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

    fireEvent.click(screen.getByText("chart"));

    expect(open).toHaveBeenCalledOnce();
  });

  it("keeps the body's own semantics, leaving keyboard users the header button", () => {
    render(
      <WidgetFrame
        title="Rows"
        state="ready"
        onBodyActivate={vi.fn()}
        bodyLabel="Show the data"
      >
        <table>
          <tbody>
            <tr>
              <td>cell</td>
            </tr>
          </tbody>
        </table>
      </WidgetFrame>
    );

    expect(screen.getByRole("cell", { name: "cell" })).toBeInTheDocument();
    expect(
      screen.getAllByRole("button", { name: "Show the data" })
    ).toHaveLength(1);
  });

  it("leaves a click on a control inside the body to that control", () => {
    const open = vi.fn();
    const retry = vi.fn();
    render(
      <WidgetFrame
        title="Traffic"
        state="error"
        onRetry={retry}
        onBodyActivate={open}
        bodyLabel="Show the data"
      />
    );
    const control = screen.getByRole("button", { name: "Retry" });

    fireEvent.click(control);

    expect(retry).toHaveBeenCalledOnce();
    expect(open).not.toHaveBeenCalled();
  });

  it("puts a button beside the title that opens what the body opens", () => {
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

    fireEvent.click(screen.getByRole("button", { name: "Show the data" }));

    expect(open).toHaveBeenCalledOnce();
  });

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
  function openFullScreen() {
    fireEvent.click(screen.getByRole("button", { name: "Full screen" }));

    return screen.getByRole("dialog", { name: "Traffic" });
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

  it("opens the widget in a dialog over the page and closes it again", async () => {
    render(
      <WidgetFrame title="Traffic" state="ready" fullscreen>
        <p>chart</p>
      </WidgetFrame>
    );

    const dialog = openFullScreen();
    expect(within(dialog).getByText("chart")).toBeInTheDocument();
    expect(
      within(dialog).queryByRole("button", { name: "Close" })
    ).not.toBeInTheDocument();

    fireEvent.click(
      within(dialog).getByRole("button", { name: "Exit full screen" })
    );

    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    );
    expect(screen.getByText("chart")).toBeInTheDocument();
  });

  it("closes the dialog before opening what the body opens", async () => {
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

    const dialog = openFullScreen();
    fireEvent.click(within(dialog).getByText("chart"));

    expect(open).toHaveBeenCalledOnce();
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    );
  });

  it("closes the dialog before opening the data from its header", async () => {
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

    const dialog = openFullScreen();
    fireEvent.click(
      within(dialog).getByRole("button", { name: "Show the data" })
    );

    expect(open).toHaveBeenCalledOnce();
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    );
  });

  it("stays open when the rest of the header is clicked", () => {
    render(
      <WidgetFrame
        title="Traffic"
        state="ready"
        fullscreen
        action={<span>All time</span>}
      >
        <p>chart</p>
      </WidgetFrame>
    );

    const dialog = openFullScreen();
    fireEvent.click(within(dialog).getByText("All time"));

    expect(screen.getByRole("dialog", { name: "Traffic" })).toBeInTheDocument();
  });
});
