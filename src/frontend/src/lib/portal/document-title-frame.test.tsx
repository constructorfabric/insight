// @vitest-environment jsdom
import { render } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { DOCUMENT_TITLE_APP } from "./document-title";
import { DocumentTitleFrame, usePageTitle } from "./document-title-frame";

function TitleProbe({ title }: { title: string | null }) {
  usePageTitle(title);
  return null;
}

function shown(pathname: string, title: string, pageTitle: string | null) {
  return (
    <DocumentTitleFrame pathname={pathname} title={title}>
      <TitleProbe title={pageTitle} />
    </DocumentTitleFrame>
  );
}

describe("DocumentTitleFrame", () => {
  beforeEach(() => {
    document.title = "Before";
  });

  it("sets the route title and restores the app title when the shell unmounts", () => {
    const { unmount } = render(shown("/portal", "Home · Insight", null));
    expect(document.title).toBe("Home · Insight");

    unmount();
    expect(document.title).toBe(DOCUMENT_TITLE_APP);
  });

  it("lets the open page replace the route title", () => {
    render(
      shown(
        "/portal/custom/delivery-overview",
        "Dashboards · Insight",
        "Delivery overview · Dashboards · Insight"
      )
    );
    expect(document.title).toBe("Delivery overview · Dashboards · Insight");
  });

  it("follows a rename of the page on screen", () => {
    const { rerender } = render(
      shown(
        "/portal/custom/delivery-overview",
        "Dashboards · Insight",
        "Delivery overview · Dashboards · Insight"
      )
    );

    rerender(
      shown(
        "/portal/custom/delivery-overview",
        "Dashboards · Insight",
        "Release health · Dashboards · Insight"
      )
    );
    expect(document.title).toBe("Release health · Dashboards · Insight");
  });

  it("drops a page title when the path changes", () => {
    const { rerender } = render(
      shown(
        "/portal/custom/delivery-overview",
        "Dashboards · Insight",
        "Delivery overview · Dashboards · Insight"
      )
    );

    rerender(shown("/portal", "Home · Insight", null));
    expect(document.title).toBe("Home · Insight");
  });
});
