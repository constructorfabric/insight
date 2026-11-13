// @vitest-environment jsdom
import { render } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { DOCUMENT_TITLE_APP } from "./document-title";
import { DocumentTitleFrame, usePageTitle } from "./document-title-frame";

function SubjectProbe({ subject }: { subject: string | null }) {
  usePageTitle(subject);
  return null;
}

function shown(pathname: string, screenPath: string, subject: string | null) {
  return (
    <DocumentTitleFrame pathname={pathname} screenPath={screenPath}>
      <SubjectProbe subject={subject} />
    </DocumentTitleFrame>
  );
}

describe("DocumentTitleFrame", () => {
  beforeEach(() => {
    document.title = "Before";
  });

  it("sets the screen title and restores Insight when the shell unmounts", () => {
    const { unmount } = render(shown("/home", "/", null));
    expect(document.title).toBe("Home · Insight");

    unmount();
    expect(document.title).toBe(DOCUMENT_TITLE_APP);
  });

  it("puts the page subject ahead of the screen name", () => {
    render(
      shown(
        "/dashboards/delivery-overview",
        "/dashboards/delivery-overview",
        "Delivery overview"
      )
    );
    expect(document.title).toBe("Delivery overview · Dashboards · Insight");
  });

  it("follows a rename of the subject on screen", () => {
    const { rerender } = render(
      shown(
        "/dashboards/delivery-overview",
        "/dashboards/delivery-overview",
        "Delivery overview"
      )
    );

    rerender(
      shown(
        "/dashboards/delivery-overview",
        "/dashboards/delivery-overview",
        "Release health"
      )
    );
    expect(document.title).toBe("Release health · Dashboards · Insight");
  });

  it("drops a subject when the path changes", () => {
    const { rerender } = render(
      shown(
        "/dashboards/delivery-overview",
        "/dashboards/delivery-overview",
        "Delivery overview"
      )
    );

    rerender(shown("/home", "/", null));
    expect(document.title).toBe("Home · Insight");
  });
});
