# Insight sample-data seeder

Python script that populates the local docker-compose stack with a
25-person demo organisation (4 teams + CEO) and per-team activity in
ClickHouse silver tables. `profiles.py` documents the roster and the
per-team source-type weights; the per-domain generators under
`generators/` document the row shapes they emit. See
[PROFILE.md](PROFILE.md) for what a freshly seeded stand actually contains
— roster, fixtures, populated metrics and capabilities.

## Run it

The stack must be up first (`./dev-compose.sh up`). Then:

```bash
./dev-compose.sh seed                       # everything
./dev-compose.sh seed identity              # just identity
./dev-compose.sh seed silver                # just silver
```

A successful run writes `manifest.json` next to this README, describing the
stand it just produced (roster, fixtures, data window, capabilities).

## Reproducing a dataset

`SEED_ANCHOR_DATE` fixes the last day carrying activity; `SEED_DAYS` sets the
window length. Pin both to reproduce a dataset exactly:

```bash
SEED_ANCHOR_DATE=2026-06-30 SEED_DAYS=60 ./dev-compose.sh seed
```

Unset (or the literal `today`), the anchor is yesterday UTC, so the developer
loop stays populated as the calendar moves. Whichever applied is recorded in
`manifest.json`, so a stand always reports how to recreate it.

## [PROFILE.md](PROFILE.md)

[`PROFILE.md`](PROFILE.md) is generated and committed. Regenerate it after any change to the
roster or the manifest builder:

```bash
python3 deploy/seed/render_profile.py            # regenerate
python3 deploy/seed/render_profile.py --check    # verify (no database needed)
```

## Develop on it

```bash
cd deploy/seed
python3 -m venv .venv                              # one-time
.venv/bin/pip install -e '.[dev]'

.venv/bin/ruff check .
.venv/bin/mypy .
```

Deps live in `pyproject.toml`: `[project.dependencies]` for runtime,
`[project.optional-dependencies].dev` for the tooling (ruff, mypy, stubs).

## Layout

| File | Role |
|------|------|
| `seed.py` | CLI entry; dispatches subcommands. |
| `profiles.py` | Demo roster + per-team activity weights. |
| `identity.py` | MariaDB seed: persons, org_chart, account_person_map. |
| `silver.py` | ClickHouse silver seed — full implementation: bronze placeholders → 8 domain generators → CH migrations → dependent-MV refresh. |
| `manifest.py` | Builds `manifest.json` — the machine-readable description of a seeded stand. |
| `golden_metrics.py` | The only source for the manifest's `golden_metrics[]`. Hand-curated. |
| `profile_md.py` | Renders `PROFILE.md` from the manifest. |
| `render_profile.py` | Regenerates / verifies `PROFILE.md`. Needs no database. |
| `PROFILE.md` | GENERATED — human-readable stand profile. Do not hand-edit. |
| `manifest.json` | GENERATED at seed time, per-stand (gitignored). |
| `Dockerfile` | One-shot image for the compose `seed-sample` service. |
| `pyproject.toml` | Package metadata, deps (runtime + dev), ruff + mypy config. |
