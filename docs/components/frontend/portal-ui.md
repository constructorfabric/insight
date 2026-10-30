# Portal UI

Services compute every figure and decide what each person sees. The portal shows what they return. Hiding a menu item is a courtesy; the services are the access control.

## Terms

- **Reach**: whose data a person may see; decided by the services from reporting lines and data grants.
- **Data grant**: permission for one kind of data: person-level figures, underlying records, cost. Each granted separately. A dashboard share is not a data grant and does not change reach.
- **Member**: signed-in person without the administrator role. **Administrator**: with it. "May edit" / "may share" below: per-dashboard flags from the services.
- **Hierarchical organisation**: has reporting lines. **Flat**: none; everyone is a peer.
- **Direction**: Development, Collaboration, Knowledge / Wiki, Sales / CRM, Support. **Lens**: one view of a direction, e.g. Flow, Quality.
- **Org scope**: the part of the organisation a lead's people views cover; also the group each person is compared against.
- **Starter dashboard**: read-only dashboard Insight provides for each connected source, refreshed nightly.
- **Planned**: shown in navigation, not built.

## Shell

- **Rail** (left): Home, Dashboards, Explore, People, Reports, Manage. Only items the viewer may use. Current item marked. Planned items muted. Below: Feedback, Settings.
- **Pane**: pages inside the selected rail item, in titled groups. Group labels uppercase. Count beside an item where it tells the reader what they will find. Home's pane titled with the product name; others with the rail item's name.
- **Toolbar**: above content, stays visible on scroll. Holds the period control. Shown on Home, Explore, People, Reports and an open dashboard; not on other Dashboards pages.
- **Phone**: no rail; one drawer holds rail items and pane; picking a page closes it. Toolbar controls on one row, scrolls sideways.
- **Settings**: What's new; show planned pages; focus mode; explanations on/off; theme (light, dark, system); viewer's name and email. Remembered in the browser.
- **Feedback**: free text, length limit, counter. Carries which screen, never its figures or names. Confirms success or failure.
- **Sign-in**: no session → redirect to sign-in. Denied → says so; retry where a retry can work.
- **Address is the view**: rail item, pane item, period, scope, folder, tags. Browser storage holds only preferences (Settings, Assistant collapsed, last-used portal period).

## Period control

One control wherever period figures appear.

- Direct: **Last 7 days**, **Last 30 days**.
- **More** menu: Yesterday, Last 7 days, Last 30 days, Last 90 days, Last month, Last quarter, All time, Custom range. Each row shows its resolved dates; current one checked. Active More preset's name replaces the word More; direct presets stay.
- Date-range button: active dates + UTC marker; opens a calendar for a custom range.
- Preset applies on click; custom range on Apply. Range over the supported maximum refused, with reason.

Date rules (same everywhere):

- Rolling presets: stated number of UTC days up to and including today.
- Yesterday: previous UTC day.
- Last month / Last quarter: previous complete calendar month / quarter.
- All time: all history.

Two periods:

- **Portal period**: Home, Explore, People, Reports. In the address. Missing → last-used portal period.
- **Dashboard period**: one per open dashboard, in its address. Missing → the dashboard's stored default.
- Changing one never changes the other. Dashboards are authored for a period and shared by link; the rest of the portal is the reader's context. One shared period would show a shared link differently per reader and let a dashboard change silently change Home.

Presets are the server's tokens; portal and services resolve them identically.

## Org scope

- **Org scope**: picked in the work chart (People pane) or the Team button on a person page. Scoped views name the scope in effect. Subtree with indirect reports: choice of direct reports only / whole subtree. No scope selector in the toolbar.

## Home

A manager, and anyone in a flat organisation, switches between **Me** and **My team** (**Organisation** when flat); My team is the default. Everyone else reads Me. Home holds its content until it is known which applies.

Me, in order:

1. **Starter dashboards for your sources** (planned): one card for each connected source. Question it answers; chip per source read, with connection state (connected / partial / missing). Actions: Open; Save as dashboard (administrators).
2. **Recent dashboards**: three most recently changed dashboards the viewer can open; left out when there are none. New dashboard button (those who may create). Cards as in the Dashboards list.
3. **At a glance**: the viewer's own tiles, each with change vs previous period and the team median.
4. **Needs attention**: the viewer's own flagged metrics.

My team, in order:

1. **Per person**: six tiles: commits, pull requests merged, tasks closed, messages sent, AI cost, AI extra usage cost. Each: change vs previous period of the same length (none for All time); "team total · N per active person".
2. **AI adoption**: people using AI, as a share of the team.
3. **Needs attention**: collapsible rows, one per metric, with people count.
4. **By direction**: one card per direction.

Each theme in the pane reads the same subject as the switch. Pane: At a glance, By direction, Trend, Attention needed, Data coverage, Contribution breakdown (My team only); Dashboards group: My dashboards with count, Starter dashboards (planned).

## Dashboards

Open to every signed-in person; each sees only what the services return for them.

### Pane

- **Browse**: All, My dashboards, Shared with me, Starred (planned), Starter (planned); counts.
- **Folders**: one row per folder, Unfiled, New folder. No counts.
- **Catalogue**: Metrics, Widgets, Datasets, Alerts.
- **Pinned**: viewer's pins.

### List

- Title "Dashboards"; filled New dashboard button with plus icon (administrators).
- Search; Tags filter; matching count; chip row mirroring Browse (pane and chips always agree).
- Cards in two columns; more load on scroll.
- Card: name; owner (if identity lets the viewer read that person) and "N ago"; visibility badge (Private / Shared with people / Shared with everyone); tags; open chevron; `···` menu with what this person may do.
- Starter card: "Insight · nightly", read-only badge; Open; Save as dashboard (administrators); no menu.

### Open dashboard

- Toolbar: the period control, bound to the dashboard period.
- Title row: title; filled Edit button (may edit); `···` menu.
- Under it: owner and visibility as on the card; chip per source with connection state; "Updated N ago".
- Widgets in stored order.
- Menu: Pin, Star (planned), Rename, Duplicate, Share, Move to folder, Tags, Delete (red, trash icon). Export snapshot, Schedule report: planned, shown to everyone. Other items only when permitted.
- Widget: title; subtitle (aggregation · grain · unit, where present); figure or chart. Headline figure shows change vs previous period. Administrators: a way to see the rows behind it. Failed widget: members see a generic failure, no cause.

### Create

- New dashboard (Home, list) opens a dialog: name; visibility (Private default, named people, everyone); start from: empty, copy of a starter, "Describe it to the Assistant" (opens the Assistant with name and visibility set). Create opens the new dashboard.
- No identifier field. No "windows offered" list: every dashboard offers every preset and has one default period.

### Sharing

- May share → share with named people or everyone, viewer or editor; change level; withdraw. Everyone never preselected. Share shown only where permitted.
- Administrators: list of dashboards and folders whose owner lost the administrator role (name, owner, last change); hand over to a current administrator.

### Folders, tags, pins, stars

As the services allow: create / rename / delete folders; put a dashboard in folders; tag; pin to the pane; star / unstar (planned).

### Catalogue, editor, alerts

- Metrics, Widgets, Datasets, Alerts: searchable paged lists. Item page: definition; a run over a chosen period where dated; what uses it.
- Editor (from Edit, not from New dashboard): form view and text view; invalid input refused per field with reason; unsaved input never discarded on refusal; removal refused while something uses the definition, naming it.
- Alert: metric, condition, period, check interval, columns, destination. Form previews the current value. On/off. Page shows latest check and notification history. No destination configured → form says so.

### Assistant

- Right column on every Dashboards page: collapsed to a strip with an open button by default; under content on narrow screens.
- Conversation survives navigation within Dashboards.
- Reply: answered or built; result rows if any; links to what it built. Refused → reason shown, question returned to the input.
- Administrators only.

## Explore

- Pane: directions with lens counts; AI & Cost: Overview, AI adoption, Cost.
- **Lens**: sections over org scope and portal period: headline with change, trends, distributions, concentration, composition, others as defined. States people covered. Source not connected → says so, no empty charts. Not built → planned card.
- **Evidence**: from any figure, band, trend point or cell: people behind it (person-level data grant); records (underlying-records data grant); drill from a person to their records in place; export CSV / Excel. No grant → "not available".
- **Repository drill-down**: row in the repositories table → that repository's headline, contributors, activity by hour, composition by branch, ownership, event histogram; breadcrumb back.
- **Organisation-grain lenses** (e.g. CI): no roster. Metrics not enabled on the installation → says so.
- **AI & Cost** (cost data grant): potential cost, active users, accepted lines, average cost per active user; cost by tool; cost and usage per person; adoption funnel; breakdown by unit or role. Seat-based and usage-based cost kept apart. Missing reading → "not tracked", never zero.

## People

- Pane: Me; My team; Roster · by role (flat: Overview; Roster). On a person page: that person's sections, dot per section showing data state. On team views: work chart: reporting tree rooted at the viewer (flat: alphabetical list), searchable; lead → their team, person → their page.
- **Person page**: name, title, department; manager if within reach; sections with data / without / no source; headline figures with trends vs the org scope; needs attention; per section: figures and the work behind them.
- **Team**: scope size; team-level summary figures; needs attention; members grid compared within the org scope, ordered by name until sorted; subset filter; open any member.
- **Roster**: everyone in reach: title, placement, status; searchable; row opens the person.

## Reports

- Report builder: metrics by family; people or organisation grain; time granularity the period supports; preview; download CSV / Excel; cancel. Cannot run → says why.
- Snapshots, PDF / HTML export, Templates: planned.

## Manage

- **Data**: Sources & connectors, Data exclusions, Org snapshots.
- **People & access**: Identities · roles & taxonomy, Access, Group management.
- **Platform**: Ingestion, Previews, AI assistant config, What's new, Platform usage, MCP servers, Config & setup, Scorecard management.
- Items shown only to people who may use them.
- Sources & connectors: per connector: state, last sync, records reported, recent syncs. Never read ≠ read with nothing recorded.
- Ingestion: row arrival rate, overall and per connector.
- Identities: review queue (merge, absorb, confirm); lookups by person or account.
- Platform usage: visits, people, pages opened over the period; who opened what; drill-downs and exports; feedback.

## Who can do what

Data visibility is reach. This table is dashboards.

| Action | Member | Administrator |
|---|---|---|
| Open shared dashboards; pin; file | yes | yes |
| Star (planned) | yes | yes |
| Create, edit, duplicate, rename, delete, tag | no | owned, or editor level |
| Share; change or withdraw shares | no | owned, or editor level via a share to them or to everyone (not via a folder) |
| Rows behind a widget | no | yes |
| Catalogue, editor, alerts, Assistant | no | yes |
| Take over orphaned dashboards and folders | no | yes |

Editor level on a member takes effect only once they hold the administrator role.

## States

Every page and every independently loaded section distinguishes:

- Loading.
- No data for this reader.
- No connected source (named).
- Withheld: group below the size a figure or comparison needs; reason shown.
- Refused: service's reason shown.
- Failed: retry.

- Single missing reading: inline "not tracked", never zero.
- Planned pages: hidden unless enabled in Settings; marked when shown; never a default landing. Installation config can hide or mark planned any page.
- Delete: confirmation stating the consequence (folder: what happens to its dashboards). Same for anything widening who sees a dashboard (a grant; moving into a shared folder). Delete drawn destructive.
- View-as: every screen says whose view it is, with a way back. Demonstration data: marked.
- Light and dark themes; colour carries the same meaning in both.
- Charts: lines and areas join readings with straight segments. A line marks every reading up to 31 points and none beyond, where hover gives the value. A reading with no neighbours always gets a dot, since it draws no line.
- Keyboard: every action reachable, visible focus; every control has an accessible name; loading, refusals, errors announced.

## Not yet

- Starred, Starter: no backend; shown as planned.
- Period control: the rules above are the target. Today the portal resolves presets in local time and ends rolling ranges yesterday, and the services have no Last 90 days token. The services add the token and the UTC rules first; the portal switches to them when it ships the control, and until then does not offer Last 90 days.
- Chart visuals (line style, legends, widget header actions): separate work.
- Four colour pairs below contrast guidance (muted text on pane and rail; input borders): designer.
- Planned: Export snapshot, Schedule report; Reports Snapshots, PDF / HTML export, Templates; Manage Access, Data exclusions, Org snapshots, Group management, MCP servers, Config & setup, Scorecard management.
- Per-installation language, date, number, currency, time zone. This release: English, UTC days.
