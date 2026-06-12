# Insight sample-data seeder

Python script that populates the local docker-compose stack with a
25-person demo organisation (4 teams + CEO) and per-team activity in
ClickHouse silver tables. `profiles.py` documents the roster and the
per-team source-type weights; the per-domain generators under
`generators/` document the row shapes they emit.

## Run it

The stack must be up first (`./dev-compose.sh up`). Then:

```bash
./dev-compose.sh seed                       # everything
./dev-compose.sh seed identity              # just identity
./dev-compose.sh seed silver                # just silver (Phase 2)
```

## Develop on it

```bash
cd insight/compose/seed
python3 -m venv .venv                              # one-time
.venv/bin/pip install -r requirements-dev.txt

.venv/bin/ruff check .
.venv/bin/mypy .
```

Pinned dep versions live in `requirements.txt` / `requirements-dev.txt`.
Re-resolve via `.venv/bin/pip install --upgrade <pkgs>` + `pip freeze`.

## Layout

| File | Role |
|------|------|
| `seed.py` | CLI entry; dispatches subcommands. |
| `profiles.py` | Demo roster + per-team activity weights. |
| `identity.py` | MariaDB seed: persons, org_chart, account_person_map. |
| `silver.py` | ClickHouse silver seed — Phase 2 placeholder. |
| `Dockerfile` | One-shot image for the compose `seed-sample` service. |
| `requirements.txt` | Pinned runtime deps. |
| `requirements-dev.txt` | Pinned dev deps (ruff, mypy, stubs). |
| `pyproject.toml` | ruff + mypy config. |
