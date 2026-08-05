# Release notes

## 0.4.69

### New UI

**We've moved to the new interface for good** — until now the new interface was an option in the sidebar. Now it's what Insight looks like.

### Dashboards

**Activity over time, by repository** — a new table and chart in Git output: commits, merged pull requests and lines, split by repository and grouped by day, week or month to match the period you picked. The 10 busiest repositories are listed, the rest roll up into "Other", and the totals always add up. Switch between chart and table, and download it as CSV or Excel.

**Metric catalog** — a new page in the sidebar lists every metric we report: what it means, its unit, which breakdowns it supports, and when it last had data. If a chart looks empty, that last column usually explains why.

**"No data" instead of a misleading zero** — collaboration, task delivery and wiki now measure the same way as Git and AI. Where something isn't measured yet, you'll see "no data" rather than a zero that looks like a real result.

### Steadier data across your connectors

Connector fixes across Jira, Bitbucket, Zoom, Claude and Microsoft 365, plus the data preparation behind them, so syncs finish and resume where they stopped instead of stalling.
