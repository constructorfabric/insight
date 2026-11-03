"""The portal shell — the lens rail a signed-in reader lands on.

Rail zones and pane items are the same shadcn `Sidebar` button, separable only
by container, so the rail is addressed through `[data-testid="lens-rail"]`.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass

from playwright.sync_api import Locator, Page

from ..steps import step

RAIL = '[data-testid="lens-rail"]'

MENU_BUTTON = '[data-slot="sidebar-menu-button"]'
PANE_BUTTONS = (
    '[data-slot="sidebar-wrapper"] > [data-slot="sidebar"] [data-slot="sidebar-menu-button"]'
)
SAFE_NAV = (
    ':not([data-testid="org-tree"] *)'
    ':not([data-slot="sidebar-group"]:has('
    '[data-slot="sidebar-group-label"]:text-is("WorkChart")) *)'
    ":not([data-planned])"
)


@dataclass(frozen=True)
class NavigationEntry:
    label: str
    expanded: str | None
    href: str | None


def navigation_entries(buttons: Locator) -> list[NavigationEntry]:
    return [
        NavigationEntry(label=entry["label"], expanded=entry["expanded"], href=entry["href"])
        for entry in buttons.evaluate_all(
            """elements => elements.map(element => ({
                label: (element.querySelector(':scope > span')?.innerText
                    ?? element.innerText).trim(),
                expanded: element.getAttribute('aria-expanded'),
                href: element.getAttribute('href')
            })).filter(entry => entry.label)"""
        )
    ]


class LensRail:
    def __init__(self, page: Page) -> None:
        self.page = page
        self.rail = page.locator(RAIL)

    def zones(self) -> Locator:
        return self.rail.locator(MENU_BUTTON)


class PortalShell:
    PATH = "/portal"

    def __init__(self, page: Page) -> None:
        self.page = page
        self.rail = LensRail(page)
        self.pane_buttons = self.safe_navigation(page.locator(PANE_BUTTONS))
        self.main = page.locator('main[data-slot="sidebar-inset"]')
        self.page_views = self.main.locator('[data-testid^="page-"]').filter(visible=True)
        self.loading_markers = self.main.locator(
            '[class*="skeleton"], .animate-pulse, [aria-busy="true"]'
        ).filter(visible=True)
        alerts = page.get_by_role("alert").filter(has_text="Something went wrong")
        statuses = page.get_by_role("status").and_(page.locator('[class*="destructive"]'))
        self.visible_errors = alerts.or_(statuses).filter(visible=True)

    def go(self) -> None:
        with step("Open Insight"):
            self.page.goto(self.PATH, wait_until="domcontentloaded")

    def safe_navigation(self, buttons: Locator) -> Locator:
        return buttons.and_(self.page.locator(SAFE_NAV)).filter(
            has_not_text=re.compile(r"^New\b"), visible=True
        )

    def entry(self, buttons: Locator, label: str) -> Locator:
        exact = re.compile(f"^{re.escape(label)}$")
        labelled = buttons.filter(has=self.page.locator(":scope > span").filter(has_text=exact))
        return labelled.or_(buttons.filter(has_text=exact)).filter(visible=True)

    def sub_buttons(self, parent: str) -> Locator:
        entry = self.entry(self.pane_buttons, parent)
        submenu = entry.locator("xpath=ancestor::li[1]").locator(
            'xpath=./ul[@data-slot="sidebar-menu-sub"] | '
            './following-sibling::*[1][self::ul and @data-slot="sidebar-menu-sub"]'
        )
        return self.safe_navigation(submenu.locator('[data-slot="sidebar-menu-sub-button"]'))

    def page_view(self, test_id: str) -> Locator:
        return self.page.get_by_test_id(test_id)

    def visible_page_ids(self) -> frozenset[str]:
        return frozenset(
            self.page_views.evaluate_all(
                "elements => elements.map(element => element.getAttribute('data-testid'))"
            )
        )

    def open_zone(self, zone: str) -> None:
        with step(f"Open the {zone} zone"):
            entry = self.entry(self.safe_navigation(self.rail.zones()), zone)
            entry.click()
            with step("Move away from the rail and wait for the pane to be uncovered"):
                entry.blur()
                self.main.hover()
                self.page.wait_for_function(
                    """({railSelector, paneSelector}) => {
                        const rail = document.querySelector(railSelector);
                        const buttons = [...document.querySelectorAll(paneSelector)]
                            .filter(button => button.checkVisibility({visibilityProperty: true}));
                        return rail && buttons.length > 0 && buttons.every(button => {
                            const rect = button.getBoundingClientRect();
                            const left = Math.max(0, rect.left);
                            const right = Math.min(innerWidth, rect.right);
                            const top = Math.max(0, rect.top);
                            const bottom = Math.min(innerHeight, rect.bottom);
                            if (right <= left || bottom <= top) return true;
                            const inset = Math.min(1, (right - left) / 2);
                            const y = (top + bottom) / 2;
                            return [left + inset, (left + right) / 2, right - inset]
                                .every(x => !rail.contains(document.elementFromPoint(x, y)));
                        });
                    }""",
                    arg={"railSelector": RAIL, "paneSelector": PANE_BUTTONS},
                    timeout=30_000,
                )

    def open_pane_entry(self, label: str) -> None:
        with step(f'Open "{label}" in the pane'):
            self.entry(self.pane_buttons, label).click()

    def expand_pane_entry(self, label: str) -> None:
        if self.entry(self.pane_buttons, label).get_attribute("aria-expanded") == "false":
            self.open_pane_entry(label)

    def open_sub_entry(self, parent: str, label: str) -> None:
        with step(f'Open "{label}" under "{parent}"'):
            self.entry(self.sub_buttons(parent), label).click()

    def first_list_href(self, prefix: str) -> str:
        handle = self.page.wait_for_function(
            r"""({prefix}) => {
                const main = document.querySelector('main[data-slot="sidebar-inset"]');
                for (const link of main?.querySelectorAll('a[href]') ?? []) {
                    const target = new URL(link.href, location.href);
                    if (link.checkVisibility({visibilityProperty: true}) &&
                        link.getBoundingClientRect().width > 0 &&
                        link.getBoundingClientRect().height > 0 &&
                        target.origin === location.origin &&
                        target.pathname.startsWith(prefix) &&
                        !/\/(new|edit)(\/|$)/.test(target.pathname)) {
                        return link.getAttribute('href');
                    }
                }
                return null;
            }""",
            arg={"prefix": prefix},
            timeout=30_000,
        )
        href = str(handle.json_value())
        handle.dispose()
        return href

    def open_list_item(self, label: str, href: str) -> None:
        with step(f"Open the first item in the {label} list"):
            self.main.locator(f"a[href={json.dumps(href)}]").filter(visible=True).first.click()
