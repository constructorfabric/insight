"""API assertions against the deployed stand, one directory per service.

    analytics/     the /api/analytics prefix, one module per path group
    identity/      the /api/identity prefix, one module per concern
    test_gateway.py    neither — the edge itself

Split by SERVICE because that is the axis along which a test's setup differs:
identity's answers depend on who is asking (the org chart, the admin row, the
kind of principal), analytics' mostly on what was created.

`test_gateway.py` stays at this level on purpose. It sweeps 401 over every
operation in `operations.py`, both services at once, because refusing an
anonymous caller is the EDGE's uniform behaviour rather than anything either
service does. Filing it under one of them would imply otherwise.

Shared across both directories, and deliberately not duplicated into them:

    conftest.py    the `api` client, scratch fixtures, the leak sweep
    operations.py  every gateway-routed operation, the sweep's universe
    scratch.py     the mutation policy and its registry
    schemas/       the response models

Still packages, all the way down. pytest imports test modules by bare basename
without `__init__.py`, and `analytics/test_metrics.py` would then collide with
any other `test_metrics.py` in the tree.
"""
