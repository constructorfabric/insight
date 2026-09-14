"""The data-path lane's library: build a fixture, drive an instance with it, read it back.

`fixture_loader`, `ref_resolver`, `schema_validator` and `records` are pure — everything
they do is computable from files, which is what lets `tests/datapath/meta` run without an
instance. The rest drive one: ClickHouse and MariaDB clients, the bronze seeder, dbt, the
enrich sidecar, the identity subjects and bindings, and the reset that clears between specs.
"""
