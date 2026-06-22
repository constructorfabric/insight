from __future__ import annotations


class GitlabAuthError(RuntimeError):
    pass


class GitlabApiError(RuntimeError):
    pass


class WindowTooLarge(RuntimeError):
    pass


class UnwindowableWindow(RuntimeError):
    pass
