# Mock source

Python standard library HTTP source for deterministic scheduling tests. Run from
the existing toolbox image. The connector is `connectors/testing/mock`.

API contracts, required environment, local install/remove commands and the five
harness slices live in
[the harness design](../../../../docs/testing/SCHEDULING-HARNESS.md).

From the repository root:

```bash
python3.12 -m pytest src/ingestion/tools/mock-source/tests -q
```
