-- Data-independent contract test for the shared `git_file_category` macro:
-- representative vendored/generated paths must classify as `vendored` (so they
-- drop out of authored line-count metrics), and representative authored paths
-- must NOT. Runs against a literal fixture set so it holds on a fresh cluster
-- with no ingested data. Any mismatch fails `dbt build` (untagged -> error).
WITH fixtures AS (
    SELECT
        tupleElement(row, 1) AS path,
        tupleElement(row, 2) AS expected
    FROM (
        SELECT arrayJoin([
            -- (path, expected_category)
            ('node_modules/react/index.js', 'vendored'),
            ('app/vendor/jquery.js', 'vendored'),
            ('src/__generated__/schema.ts', 'vendored'),
            ('dist/bundle.min.js', 'vendored'),
            ('proto/user.pb.go', 'vendored'),
            ('lib/model.g.dart', 'vendored'),
            ('go.sum', 'vendored'),
            ('frontend/pnpm-lock.yaml', 'vendored'),
            ('services/.venv/lib/site-packages/foo.py', 'vendored'),
            ('api/types.d.ts', 'vendored'),
            ('package-lock.json', 'vendored'),
            ('backend/Cargo.lock', 'vendored'),
            ('src/app/main.py', 'code'),
            ('src/components/Button.tsx', 'code'),
            ('lib/vendorize.py', 'code'),
            ('src/building_blocks/x.js', 'code'),
            ('README.md', 'docs'),
            ('config/settings.yaml', 'config'),
            ('tests/test_main.py', 'test')
        ]) AS row
    )
)
SELECT
    path,
    expected,
    {{ git_file_category('path') }} AS actual
FROM fixtures
WHERE {{ git_file_category('path') }} != expected
