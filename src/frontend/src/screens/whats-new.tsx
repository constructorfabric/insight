import { ChevronDown } from "lucide-react";
import { Trans, useTranslation } from "react-i18next";

import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { SidebarTrigger } from "@/components/ui/sidebar";

// The release reads as sections, the way the written release notes are grouped.
// Each section is one row of the release card, styled like the archived
// releases below: its name sits in the left column, its entries on the right.
const RELEASE_SECTIONS = [
  { id: "new_ui", itemKeys: ["new_interface"] },
  {
    id: "dashboards",
    itemKeys: ["activity_over_time", "metric_catalog", "honest_no_data"],
  },
  { id: "connectors", itemKeys: ["steadier_data"] },
] as const;

// Each entry states today's limitation before the plan that addresses it —
// there is no separate "still on our list" section, which only restated these.
// Grouped and rendered like a release, so what is coming reads the same way as
// what shipped.
const COMING_SECTIONS = [
  { id: "trust", itemKeys: ["drill_down"] },
  { id: "platform", itemKeys: ["people_matching", "role_cohorts"] },
] as const;

// Past releases stay on the page so a reader can see the whole history, newest
// first, and are grouped into sections the same way the current one is. Each
// entry keeps the improvements it announced; its "still on our list" and
// "coming next" lists are not repeated, since the current release's lists
// supersede them.
const PAST_RELEASES = [
  {
    id: "release_2026_07_13",
    sections: [
      {
        id: "team_dashboards",
        itemKeys: ["direct_reports_toggle", "member_expand_full_metrics"],
      },
      {
        id: "git",
        itemKeys: [
          "bitbucket_prs",
          "consistent_git_metrics",
          "readable_git_charts",
        ],
      },
      { id: "task_delivery", itemKeys: ["jira_task_delivery"] },
      { id: "collaboration", itemKeys: ["zoom_data"] },
      {
        id: "ai_adoption",
        itemKeys: ["ai_adoption_graphs", "claude_code_cost"],
      },
    ],
  },
] as const;

type ReleaseSection = {
  id: string;
  itemKeys: readonly string[];
};

// Translations carry <strong>/<i> emphasis; restore foreground weight inside
// muted copy so the emphasized fragments read as in the source notes.
const EMPHASIS =
  "[&_strong]:font-semibold [&_strong]:text-foreground [&_i]:italic";

// One improvement per key, shared by the current release and the archived ones
// — only the translation prefix differs.
function ImprovementList({
  itemsKey,
  itemKeys,
}: {
  itemsKey: string;
  itemKeys: readonly string[];
}) {
  const { t } = useTranslation();

  return (
    <>
      {itemKeys.map((key) => (
        <article key={key}>
          <h4 className="text-base leading-snug font-semibold">
            {t(`${itemsKey}.${key}.title`)}
          </h4>
          <p
            className={`mt-1 text-sm leading-relaxed text-muted-foreground ${EMPHASIS}`}
          >
            <Trans i18nKey={`${itemsKey}.${key}.description_html`} />
          </p>
        </article>
      ))}
    </>
  );
}

// A release body: one row per section, its name in the left column and its
// entries on the right. Both the current release and the archived ones use it,
// so a reader sees the same shape wherever they look.
function ReleaseSections({
  baseKey,
  sections,
}: {
  baseKey: string;
  sections: readonly ReleaseSection[];
}) {
  const { t } = useTranslation();

  return (
    <>
      {sections.map((section) => (
        <div
          key={section.id}
          className="grid gap-x-5 gap-y-3 p-4 sm:grid-cols-[10rem_1fr] sm:p-5"
        >
          <h4 className="pt-0.5 text-[11px] font-semibold tracking-wide text-muted-foreground uppercase">
            {t(`${baseKey}.sections.${section.id}.title`)}
          </h4>
          <div className="flex flex-col gap-4">
            <ImprovementList
              itemsKey={`${baseKey}.items`}
              itemKeys={section.itemKeys}
            />
          </div>
        </div>
      ))}
    </>
  );
}

export function WhatsNewScreen() {
  const { t } = useTranslation();

  return (
    <>
      <header className="sticky top-0 z-20 flex items-center gap-3 border-b bg-background/95 px-4 py-3 backdrop-blur-sm">
        <SidebarTrigger />
        <h1 className="text-xl font-semibold tracking-tight">
          {t("whats_new.nav_label")}
        </h1>
      </header>

      <main className="flex flex-1 flex-col p-4 md:p-6">
        <div className="mx-auto flex w-full max-w-3xl flex-col gap-8 pb-12">
          <section>
            <p className="text-[11px] font-semibold tracking-widest text-muted-foreground uppercase">
              {t("whats_new.eyebrow")}
            </p>
            <h2 className="mt-3 text-3xl font-bold tracking-tight text-balance">
              {t("whats_new.title")}
            </h2>
            <div className="mt-4 flex flex-wrap gap-x-6 gap-y-2 font-mono text-xs text-muted-foreground">
              <span>
                <span className="font-semibold text-foreground">
                  {t("whats_new.stamp.release_label")}
                </span>{" "}
                {t("whats_new.stamp.release")}
              </span>
              <span>
                <span className="font-semibold text-foreground">
                  {t("whats_new.stamp.highlights_label")}
                </span>{" "}
                {t("whats_new.stamp.highlights")}
              </span>
              <span>
                <span className="font-semibold text-foreground">
                  {t("whats_new.stamp.focus_label")}
                </span>{" "}
                {t("whats_new.stamp.focus")}
              </span>
            </div>
          </section>

          <section>
            <h3 className="text-[11px] font-semibold tracking-widest text-muted-foreground uppercase">
              {t("whats_new.improvements_label")}
            </h3>
            <div className="mt-3 divide-y overflow-hidden rounded-lg border bg-card">
              <ReleaseSections
                baseKey="whats_new"
                sections={RELEASE_SECTIONS}
              />
            </div>
          </section>

          <section>
            <h3 className="text-[11px] font-semibold tracking-widest text-muted-foreground uppercase">
              {t("whats_new.coming.label")}
            </h3>
            <div className="mt-3 divide-y overflow-hidden rounded-lg border bg-card">
              <ReleaseSections
                baseKey="whats_new.coming"
                sections={COMING_SECTIONS}
              />
            </div>
          </section>

          <section>
            <h3 className="text-[11px] font-semibold tracking-widest text-muted-foreground uppercase">
              {t("whats_new.history_label")}
            </h3>
            <div className="mt-3 flex flex-col gap-2.5">
              {PAST_RELEASES.map((release) => (
                <Collapsible
                  key={release.id}
                  className="overflow-hidden rounded-lg border bg-card"
                >
                  <CollapsibleTrigger
                    render={
                      <button
                        type="button"
                        className="group flex w-full cursor-pointer items-center justify-between gap-3 px-4 py-3 text-left transition-colors hover:bg-accent/40"
                      />
                    }
                  >
                    <span className="flex flex-col">
                      <span className="flex flex-wrap items-baseline gap-x-2">
                        <span className="text-sm font-semibold">
                          {t(`whats_new.history.${release.id}.title`)}
                        </span>
                        <span className="font-mono text-xs text-muted-foreground">
                          {t(`whats_new.history.${release.id}.release`)}
                        </span>
                      </span>
                      <span className="mt-0.5 text-xs text-muted-foreground">
                        {t(`whats_new.history.${release.id}.summary`)}
                      </span>
                    </span>
                    <ChevronDown className="size-4 shrink-0 text-muted-foreground transition-transform group-data-[panel-open]:rotate-180" />
                  </CollapsibleTrigger>
                  <CollapsibleContent className="border-t">
                    <div className="divide-y">
                      <ReleaseSections
                        baseKey={`whats_new.history.${release.id}`}
                        sections={release.sections}
                      />
                    </div>
                  </CollapsibleContent>
                </Collapsible>
              ))}
            </div>
          </section>

          <footer className="border-t pt-4 font-mono text-xs text-muted-foreground">
            {t("whats_new.footer")}
          </footer>
        </div>
      </main>
    </>
  );
}
