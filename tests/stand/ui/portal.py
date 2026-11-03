from .checks import mark_page_requests, verify_page
from .page_watch import PageWatch
from .pages.portal_shell import PortalShell
from .steps import step


class PortalPage:
    def __init__(self, shell: PortalShell, watch: PageWatch) -> None:
        self.shell = shell
        self.watch = watch
        self.page_name = "portal"

    def open_zone(self, zone: str) -> None:
        mark_page_requests(self.shell, self.watch)
        self.shell.open_zone(zone)

    def open_pane_entry(self, label: str) -> None:
        mark_page_requests(self.shell, self.watch)
        self.shell.open_pane_entry(label)
        self.page_name = label

    def open_sub_entry(self, parent: str, label: str) -> None:
        self.shell.expand_pane_entry(parent)
        mark_page_requests(self.shell, self.watch)
        self.shell.open_sub_entry(parent, label)
        self.page_name = label

    def open_first_item(self, label: str, prefix: str) -> None:
        with step(f"Find the first item in the {label} list"):
            href = self.shell.first_list_href(prefix)
        mark_page_requests(self.shell, self.watch)
        self.shell.open_list_item(label, href)
        self.page_name = label.removeprefix("All ").removesuffix("s").lower()

    def verify_page_works(self, test_id: str) -> None:
        verify_page(self.shell, self.watch, test_id, self.page_name)
