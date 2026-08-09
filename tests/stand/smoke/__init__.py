"""Post-deploy smoke checks against a DEPLOYED (cluster) stand.

Four checks, in narrowing order — the edge answers, a person can authenticate,
the session names that person, and the seeded data is actually queryable. They
exist to gate a deployment: after CI publishes an umbrella chart, upgrades the
test stand to that exact version and re-seeds it, this directory is what says
"a user can log in and see data" or names what broke instead.

Why a sibling of `api/` and `ui/` rather than a module inside them:

* `api/` is a CONTRACT suite. It asserts status codes per operation, validates
  every body against the generated OpenAPI models, feeds the per-operation
  coverage gate, and carries a scratch-mutation policy with a session-scoped
  leak sweep. A deploy gate must not inherit any of that: it has to stay green
  when a model gains a field, and it must never write to the stand it is
  smoke-testing.
* `ui/` needs a browser. A deploy gate should fail in seconds on an HTTP
  answer, not minutes later on a screenshot; the browser journeys already
  cover the rendered half against the compose stand.

Everything else is shared. The manifest reader, `LoginSession`, `ApiClient` and
the persona helpers all come from `tests/lib/insight_stand`; nothing here
re-implements a transport, a login or a credential lookup.

Still a package, all the way down — pytest imports test modules by bare
basename without `__init__.py`, and this directory's module names would collide
with the ones under `api/` and `ui/`.
"""
