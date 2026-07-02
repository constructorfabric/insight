# UI render-contract tests

Gold has two contracts: the **API value** and the **UI render**. This suite covers
the render half — `displayed == documented_transform(api_value)` — so a tile can't
silently round, drop, or mislabel a value.

| File | Layer | Runs where | Needs |
|---|---|---|---|
| `render_contract.py` | the transform spec (pure fn) | anywhere | — |
| `test_render_contract.py` | **unit** | `pytest` / `python test_render_contract.py` | nothing |
| `test_live_render_e2e.py` | **e2e** (DOM vs API) | against deployed app | `INSIGHT_BASE_URL`, `INSIGHT_STORAGE_STATE`, playwright |

## Unit (no infra)

```bash
pytest tests/ui_render_contract/test_render_contract.py -v
```

Encodes the rounding-ownership and null/ComingSoon rules: 98.8→"99%", 4.4→4 / 4.6→5,
null→"—" (not "0%"), not-ingested→"ComingSoon" (not "0"), "0 tasks" (not "0tasks").

## e2e (live, auth-gated)

The app is behind Entra+MFA, so the login is **not** automated. Capture an auth
state once, by hand:

```bash
playwright codegen --save-storage=auth.json https://insight-dev.constr.dev/
# log in (incl. MFA) in the browser that opens, then close it
INSIGHT_BASE_URL=https://insight-dev.constr.dev \
INSIGHT_STORAGE_STATE=auth.json \
INSIGHT_PERSON=<login-email> \
pytest tests/ui_render_contract/test_live_render_e2e.py -v
```

The e2e captures the page's own `/metrics/queries` responses and asserts each KPI
tile matches the contract. Assertions for not-ingested / null metrics are `xfail`
with `constructorfabric/insight#1337` — they pass (as xfail) until the FE is fixed,
then flip to a hard failure so the regression can't return.
