# Frontend agent notes

React 19 SPA (Vite, TanStack Router + Query, Tailwind 4 + shadcn/ui, MSW for mocks).
Run every command from this directory — `pnpm` is scoped here, not to the repository root.

## Commands

| Task | Command |
|---|---|
| Dev server | `pnpm dev` |
| Lint | `pnpm lint` |
| Typecheck | `pnpm typecheck` |
| Unit tests | `pnpm test` |
| Full CI suite (unit + story, merged coverage) | `pnpm test:coverage:ci` |

## What CI enforces

`ci.yml`'s `js` job runs lint, typecheck and `test:coverage:ci`, then hands one Cobertura report to
the repository-wide `coverage-gate`. Both vitest projects run in that single pass, so a component
covered only by a story still counts.

New and changed lines must reach 80% coverage. The component's overall floor is lower while the
suite is built out — see `overall_min` in `scripts/ci/components.py`. Paths excluded from
`vitest.config.ts`'s coverage config are invisible to both gates rather than scored zero, so adding
a file there silently removes it from enforcement.

The story suite runs in a browser, so CI executes this job inside the Playwright image; the image
tag must track the `playwright` version in `package.json`.

## Conventions

- Route files under `src/routes/` stay thin — they wrap a screen component and nothing else.
- `src/components/ui/` is vendored shadcn/ui. Regenerate rather than hand-edit.
- API access goes through the clients in `src/api/` and the hooks in `src/queries/`; components do
  not call `fetch` directly.
- Metric semantics are server-owned. Do not invent user-facing metric names, descriptions or units
  here — take them from the API response.
