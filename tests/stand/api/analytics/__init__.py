"""Contract tests for the analytics service, one module per path group.

Reached at `/api/analytics/*` through the gateway. Fixtures, the scratch
policy and the response models are the api/ package's, one level up — a
resource created here is deleted by the same session-scoped leak sweep that
covers identity.
"""
