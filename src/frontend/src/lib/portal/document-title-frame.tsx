/* eslint-disable react-refresh/only-export-components -- the frame and
   usePageTitle share one override slot; splitting them would hide that. */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";

import { useRouterState } from "@tanstack/react-router";

import {
  DOCUMENT_TITLE_APP,
  documentTitle,
  titleScreenPath,
} from "@/lib/portal/document-title";
import { resolveZoneItem } from "@/lib/portal/nav-model";
import { usePortalItem } from "@/lib/portal/portal-nav";
import { useActiveZone } from "@/lib/portal/use-active-zone";
import { useSelectedPersonSection } from "@/lib/portal/use-person-sections";

interface TitleSubject {
  pathname: string;
  subject: string;
}

const TitleSubjectContext = createContext<(subject: string | null) => void>(
  () => {}
);

export function PortalDocumentTitle({ children }: { children: ReactNode }) {
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  });
  const search = useRouterState({
    select: (state) => state.location.searchStr,
  });
  const { activeZone } = useActiveZone();
  const rawItem = usePortalItem();
  const section = useSelectedPersonSection();
  const item =
    activeZone === "person" ? section : resolveZoneItem(activeZone, rawItem);

  return (
    <DocumentTitleFrame
      pathname={pathname}
      screenPath={titleScreenPath({
        pathname,
        search: search ?? "",
        zone: activeZone,
        item,
      })}
    >
      {children}
    </DocumentTitleFrame>
  );
}

export function DocumentTitleFrame({
  pathname,
  screenPath,
  children,
}: {
  pathname: string;
  screenPath: string;
  children: ReactNode;
}) {
  const [override, setOverride] = useState<TitleSubject | null>(null);
  // INVARIANT: a subject names the path it was published on. A path change
  // drops it in the same render, before the leaving page's effect runs.
  const subject = override?.pathname === pathname ? override.subject : null;
  const shown = documentTitle(screenPath, subject);

  useEffect(() => {
    document.title = shown;
  }, [shown]);

  useEffect(() => {
    return () => {
      document.title = DOCUMENT_TITLE_APP;
    };
  }, []);

  const publish = useCallback(
    (next: string | null) => {
      const trimmed = next?.trim();
      setOverride(trimmed ? { pathname, subject: trimmed } : null);
    },
    [pathname]
  );

  return (
    <TitleSubjectContext.Provider value={publish}>
      {children}
    </TitleSubjectContext.Provider>
  );
}

export function usePageTitle(subject: string | null): void {
  const publish = useContext(TitleSubjectContext);

  useEffect(() => {
    publish(subject);
    return () => publish(null);
  }, [publish, subject]);
}
