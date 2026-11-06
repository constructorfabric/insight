# Metric Definitions

Implementation of the metrics domain. The system contract and the authoring
guide live in `registry.yaml` alongside this file.

Read that document before adding or changing metrics, metric seeds,
observation sources, or gold metric views.

Declared dimension coverage is diagnostic only. Each validation sweep warns
for each uncovered dimension, naming the metric key and dimension. Coverage
probe failures also warn; neither changes the outcome of the other validation
probes. Rows missing a dimension use the unknown bucket in breakdowns and are
excluded by filters on that dimension.

The `dimension_not_covered` error code remains supported for stored statuses
from earlier versions. The next definitive validation sweep replaces those
statuses using the other probes.
