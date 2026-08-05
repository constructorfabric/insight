"""Contract tests for the identity-resolution service, by concern.

Reached at `/api/identity/*` through the gateway. Split from analytics
because the two differ in what a test has to arrange: identity's answers
depend on WHO is asking — the org chart, the admin row, the kind of
principal — while analytics' depend mostly on what was created.
"""
