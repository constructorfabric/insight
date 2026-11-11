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
  pageDocumentTitle,
  type PageTitleInput,
} from "@/lib/portal/document-title";
import {
  usePortalDir,
  usePortalItem,
  usePortalLens,
} from "@/lib/portal/portal-nav";
import { usePortalSearch } from "@/lib/portal/portal-search";
import { useActiveZone } from "@/lib/portal/use-active-zone";
import { useVisibilityPolicy } from "@/queries/identity-me";

interface TitleOverride {
  pathname: string;
  title: string;
}

const TitleOverrideContext = createContext<(title: string | null) => void>(
  () => {}
);

export function PortalDocumentTitle({ children }: { children: ReactNode }) {
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  });
  const { activeZone } = useActiveZone();
  const item = usePortalItem();
  const dir = usePortalDir();
  const lens = usePortalLens();
  const { mode } = usePortalSearch();
  const { isFlat } = useVisibilityPolicy();
  const input: PageTitleInput = {
    pathname,
    zone: activeZone,
    item,
    dir,
    lens,
    mode,
    peopleFlat: isFlat,
  };

  return (
    <DocumentTitleFrame pathname={pathname} title={pageDocumentTitle(input)}>
      {children}
    </DocumentTitleFrame>
  );
}

export function DocumentTitleFrame({
  pathname,
  title,
  children,
}: {
  pathname: string;
  title: string;
  children: ReactNode;
}) {
  const [override, setOverride] = useState<TitleOverride | null>(null);
  // INVARIANT: a published title names the path it was published on. A path
  // change drops it in the same render, before the leaving page's effect runs.
  const shown = override?.pathname === pathname ? override.title : title;

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
      setOverride(next === null ? null : { pathname, title: next });
    },
    [pathname]
  );

  return (
    <TitleOverrideContext.Provider value={publish}>
      {children}
    </TitleOverrideContext.Provider>
  );
}

export function usePageTitle(title: string | null): void {
  const publish = useContext(TitleOverrideContext);

  useEffect(() => {
    publish(title);
    return () => publish(null);
  }, [publish, title]);
}
