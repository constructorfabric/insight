import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import {
  scrollEndIntoView,
  scrollEndOutOfView,
} from "@/test/intersection-observer";

import { DefinitionCount, MoreDefinitions } from "./definition-paging";

describe("<DefinitionCount>", () => {
  it("counts the catalogue when nothing narrows it", () => {
    render(<DefinitionCount total={115} noun="metrics" searching={false} />);

    expect(screen.getByText("115 metrics")).toBeInTheDocument();
  });

  it("counts the matches while a search narrows it", () => {
    // "15 metrics" would read as the whole catalogue having shrunk.
    render(<DefinitionCount total={15} noun="metrics" searching />);

    expect(screen.getByText("15 matching")).toBeInTheDocument();
  });

  it("says nothing before the first page lands", () => {
    const { container } = render(
      <DefinitionCount total={undefined} noun="metrics" searching={false} />
    );

    expect(container).toBeEmptyDOMElement();
  });
});

describe("<MoreDefinitions>", () => {
  it("asks for the next page when the end of the list comes into view", () => {
    scrollEndOutOfView();
    const onMore = vi.fn();

    render(
      <MoreDefinitions hasMore isFetchingMore={false} onMore={onMore} />
    );
    expect(onMore).not.toHaveBeenCalled();

    scrollEndIntoView();

    expect(onMore).toHaveBeenCalled();
  });

  it("asks for nothing once every page is read", () => {
    scrollEndOutOfView();
    const onMore = vi.fn();

    render(
      <MoreDefinitions hasMore={false} isFetchingMore={false} onMore={onMore} />
    );
    scrollEndIntoView();

    expect(onMore).not.toHaveBeenCalled();
  });

  it("asks once while a page is already on its way", () => {
    scrollEndOutOfView();
    const onMore = vi.fn();

    render(<MoreDefinitions hasMore isFetchingMore onMore={onMore} />);
    scrollEndIntoView();

    expect(onMore).not.toHaveBeenCalled();
  });
});
