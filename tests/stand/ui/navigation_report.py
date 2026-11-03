"""Identify dashboard folders and merge navigation discoveries by rendered page identity."""

from collections.abc import Iterable
from dataclasses import dataclass
from urllib.parse import parse_qs, urlsplit


@dataclass(frozen=True)
class NavigationPage:
    path: tuple[str, ...]
    page_ids: frozenset[str]


def is_dashboard_folder(href: str | None) -> bool:
    if not href:
        return False
    target = urlsplit(href)
    folders = parse_qs(target.query).get("folder", [])
    return target.path.rstrip("/") == "/portal/custom" and any(
        folder != "unfiled" for folder in folders
    )


def new_page_paths(pages: Iterable[NavigationPage], covered_ids: Iterable[str]) -> list[str]:
    seen_ids = set(covered_ids)
    paths: set[str] = set()
    for page in pages:
        if page.page_ids and page.page_ids <= seen_ids:
            continue
        seen_ids.update(page.page_ids)
        paths.add(" > ".join(page.path))

    return sorted(paths)
