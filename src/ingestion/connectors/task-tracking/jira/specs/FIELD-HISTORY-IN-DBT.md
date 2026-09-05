# Field history in dbt

Design for replacing the Rust `jira-enrich` binary with dbt models that derive
`staging.jira__task_field_history` from bronze.

Status: implemented up to the cutover. The models of §2–§9 are built alongside the Rust binary, which stays the silver producer until §10 is carried out.

## 1. Why

Two defects in the current pipeline share one root cause: the shape of a Jira
field is guessed rather than read from field metadata.

**Truncated snapshot.** `jira__issue_field_snapshot` emits a hardcoded list of
about ten generic fields. The Rust `reconstruct_initial` seeds per-issue state
from that snapshot and walks the changelog backwards, so a field absent from the
snapshot reaches history only if it also appears in the changelog. A field set
at issue creation and never changed produces no row at all — no
`synthetic_initial`, no `changelog`. Adding a field to the snapshot does not
repair existing issues either: an issue holding any history row never re-enters
the `bootstrap` path, because both the event loop and the Category C tail skip
anything present in `last_state`.

**Content-sniffed delta shapes.** `core/jira.rs::to_delta` decides how to read a
multi-value changelog item by inspecting the item's own values: it matches on
`value_to` / `value_from` (the id side) and detects the legacy list shape by
searching for `", "` in the display side. Jira emits at least four shapes, and
two of them break that assumption:

- Fields of type `com.atlassian.jira.plugin.system.customfieldtypes:labels`, and
  the system `labels` field, carry no id side at all: `from` and `to` are NULL
  and the whole list sits in `fromString` / `toString`, space-separated.
  `multi_to_add_or_remove` matches `value_to`, then `value_from`, finds neither,
  and returns `None` — the event is discarded.
- `multiselect`, `multicheckboxes`, `multiversion`, `people` and
  `multiuserpicker` emit a bracketed id list (`[id, id]`) with displays joined by
  a bare comma. The `", "` probe reads only the display side, so it does not
  fire; `value_to` is non-NULL, so the row becomes a single `Add` whose id is the
  literal bracketed string, and the removal implied by `from` is lost.

Because the probe is content-based, one field can take different branches on
different rows depending on whether a value happens to contain `", "`.

Both defects disappear if the shape of a field is resolved from
`bronze_jira.jira_fields` instead of from the value.

## 2. Approach

Derive the whole table in dbt, per `(issue, field)`, from three bronze inputs.

| input | role |
|---|---|
| `bronze_jira.jira_fields` | field metadata — the classifier's only source |
| `bronze_jira.jira_issue` | current value of every field, as JSON |
| `bronze_jira.jira_issue_history` | changelog items |
| `staging.jira__task_field_history` | output |

The reconstruction walks each field's history from newest to oldest, reverse-
applying every event to the current value, so the oldest emitted row is the
field's value at issue creation. Each `(issue, field)` pair is independent,
which is what makes the incremental strategy cheap (§7).

### 2.1 A backward fold is needed for one class only

Most events are self-describing and need no accumulation:

- **Single-value fields** carry both `from` and `to` on every item. The state
  after an event is its `to`, the state before is its `from`. No fold.
- **List-snapshot multi-value fields** (labels-type, the bracketed-id types, and
  the legacy `", "` types) carry the *complete* list on both sides. No fold.
- **Element-wise multi-value fields** (`components`, `fixVersions`, `versions`,
  `issuelinks`, `attachment`) emit one item per element added or removed. Only
  these accumulate, and only these need a real fold.

So the expensive path applies to a minority of fields, and within a
`(issue, field)` group the event count is small — a `groupArray` ordered by
`(event_at, changelog_id)` folded with `arrayFold` is sufficient. No recursive
CTE and no self-join.

The fold is what the implementation does, and the detour is worth recording
because it looked equivalent. Window set arithmetic —

    state after event k = (initial ∪ additions up to k) \\ removals up to k
    initial             = (current ∪ every removal) \\ every addition

— is correct only while each element is added at most once and removed at most
once. An element added, removed and **added again** stays subtracted forever,
because it is in "every removal", so the field's newest state loses a value the
issue still holds. Set arithmetic cannot express a cycle. What is written now
collects the ordered `(action, element)` list per `(issue, field)`, obtains the
initial state by folding it backwards from the current value with each operation
inverted, and each intermediate state by folding the prefix forwards.

Two properties of the fold are load-bearing:

- the full operation list must be collected **in order**. A window without an
  `ORDER BY` collects in an arbitrary one, and undoing operations out of order
  is wrong for exactly the cycles the fold exists to handle;
- an `add` **replaces** the element's entry rather than appending, so a re-add
  carries the display the later event rendered and the element is still counted
  once.

**Open: `created_at` and `changelog_id` can disagree on the order of an issue's
own events.** An issue was observed whose latest-by-timestamp item is its
lowest-numbered one, and only the id order reconciles with the value the issue
holds — plausible for history carried across an instance migration, where ids
were reassigned on import. The model sorts by `(event_at, changelog_id)`. Which
to trust is not decided here; the round trip is what makes the disagreement
visible.

## 3. Field-kind classifier

One model, `jira__task_field_kind`, reduces Jira's field-type zoo to a closed
enum. It reads `bronze_jira.jira_fields` and nothing else.

### 3.1 Structure first, plugin keys only to disambiguate

The three schema columns are not equally portable:

- `schema_type` and `schema_items` are a **closed** set defined by the Jira REST
  field schema. Every Jira instance uses the same values.
- `schema_custom` is **open**: any app can add a type key. Another instance with
  a different set of installed apps presents keys this repository has never seen.

Rules must therefore key on structure, and reach for `schema_custom` only where
structure alone cannot decide. A classifier built the other way around — matching
`schema_custom LIKE '%:multiselect'` and friends — sends every unseen app field
to `UNKNOWN` and fails the run on any instance but the one it was written
against.

Where structure does not decide, the discriminator is not the app's identity but
whether the field is **system or custom**, i.e. whether `schema_custom` is
populated at all. System fields have first-class changelog support and emit one
item per element; custom fields are rendered by a generic serializer and emit the
whole list. That is a product-level behaviour, not an instance-level one.

**INVARIANT: no `customfield_` literal may appear in the classifier.** Field ids
are instance-specific; type constants are not. This is enforced by a test.

| `field_kind` | matched by (structure first) | JSON value shape | changelog shape |
|---|---|---|---|
| `obj` | `schema_type` in status / priority / resolution / issuetype / project | `{"id","name"}` | Set |
| `obj` | `field_id = 'parent'` — all three schema columns are empty for it | `{"id","key"}` | Set |
| `option` | `schema_type = option` | `{"id","value"}` | Set |
| `user` | `schema_type = user` | `{"accountId","displayName"}` | Set |
| `scalar` | `schema_type` in string / number / date, not long text | `"text"`, `5`, `"2026-01-05"` | Set |
| `duration` | `field_id` in timeestimate / timeoriginalestimate | `28800`, or `0`/`null` for none | Set, `0` for none (§3.5) |
| `datetime` | `schema_type = datetime` | `"2026-01-05T07:00:00.000+0000"` | Set, same instant spelled without milliseconds |
| `long_text` | `field_id` in description / environment, or `schema_custom` ends `:textarea` | ADF document | Set, rendered, possibly truncated |
| `string_array` | `schema_type = array` **and** `schema_items = 'string'` | `["a","b"]` | Snapshot, space-separated |
| `obj_array` | `schema_type = array`, `schema_items` in component / version / attachment, `schema_custom` **empty** | `[{"id","name"}]` | Add / Remove |
| `link_array` | `schema_type = array`, `schema_items = issuelinks`, `schema_custom` **empty** | `[{"id", "outwardIssue": {"key"}}]` | Add / Remove, naming the LINKED ISSUE by key |
| `option_array` | `schema_type = array`, any other `schema_items`, or the same items with `schema_custom` populated | `[{...}]` | Snapshot, `[id, id]` + comma-joined displays |
| `legacy_list` | `schema_custom` ends `:gh-sprint` — the one named exception | `[{...}]` | Snapshot, `", "`-separated |
| `issue_ref` | `schema_custom` ends `:gh-epic-link` or `:jpo-custom-field-parent` | `"PROJ-1"` — the referenced issue's key | Set, numeric id + key display |
| `ignored` | `schema_type` in progress / watches / votes / statusCategory / securitylevel / timetracking / issuerestriction / comments-page; `schema_items = 'worklog'`; `field_id` in issuekey / thumbnail | container or aggregate | not a field state |
| `ignored` | `schema_custom` ends `:gh-lexo-rank`, `:timeinstatus`, `:devsummarycf`, `:vulnerabilitycf` | ordering artifact or app-computed | decided, not measured |
| `ignored` | app and JSM single-value types: `schema_type` in option-with-child / option2 / team / atlas-project / sd-request-lang / sd-approvals / sd-feedback / sd-customerrequesttype | app-specific | decided, not measured |

Four consequences of matching on structure rather than on plugin keys:

**`schema_items = 'string'` unifies the labels family.** The system `labels`
field and every custom labels-type field are both `array` / `string`, and both
serialize as a space-separated list. There is no "system labels is special"
exception — the item type alone decides.

**The system-versus-custom discriminator is needed for one item type.**
`fixVersions` and `versions` are `array` / `version` with no `schema_custom`;
a custom multi-version picker is `array` / `version` *with* one. Same structure,
different changelog shape, so `schema_custom` being populated is what separates
element-wise from full-list here. The rule is stated generally because the same
split would apply to any future custom picker over a system item type.

**`ignored` is structural.** Containers and computed aggregates are identified by
`schema_type`, not by a list of field names — one branch covers progress, watch
and vote counts, time tracking, the comments page, issue restrictions, security
level, status category, and app-computed `any` fields. Only `issuekey` and
`thumbnail` need naming, because they carry no schema at all.

**`long_text` has no structural discriminator from `scalar`.** `summary`,
`description` and `environment` are all `string` with both other columns empty;
only the field name separates a title from an ADF body. Naming `description` and
`environment` is nonetheless portable: they are Jira system field names, fixed
across every instance. This is exactly why the invariant bans `customfield_`
literals specifically rather than all literals — instance-specific ids are the
hazard, product constants are not.

**`datetime` is separated from `scalar` because the two sides spell it
differently.** The issue resource writes the instant with milliseconds and the
changelog without, so compared as text one moment reads as two values and every
datetime field an issue carries fails the round trip. A date-only field is
**not** in this kind: both sides already spell it the same way, and reducing it
to an instant would break a value that reconciles today. The discriminator is a
`schema_type` value, so it stays structural.

**`schema_type = 'any'` is the one bucket that must be resolved by
`schema_custom`.** `any` is not a structure — it is Jira declaring that an app
owns the type, so structure carries no information at all. The bucket mixes
fields with heavy changelog traffic (an epic link, a portfolio parent link)
with genuinely computed ones (time-in-status, development summary), and a
structural rule that ignored the whole bucket would silently drop the former —
the exact defect class this design removes. Matching on `schema_custom` here is
legitimate: the keys belong to bundled Atlassian apps and are product constants,
not instance-specific ids. An `any` field whose `schema_custom` is **not** in the
known list resolves to `UNKNOWN`, so a new app field that carries real traffic
fails loudly instead of disappearing.

**`issue_ref` carries an id-space asymmetry.** The issue JSON holds the
referenced issue's *key* as a bare string, while the changelog holds the
referenced issue's *numeric id* with the key as its display. The two sides do not
reconcile on ids at all, so the **key is used as both value and identifier** on
both sides and the numeric id is discarded; it stays recoverable from
`bronze_jira.jira_issue_keys`.

The first version of this kind emitted an empty `value_ids` and kept only the
display, which reads well but breaks `assert_value_arrays_same_length` — an
existing invariant on the class contract requiring the two arrays to be
parallel. The invariant is right and the design was wrong. This asymmetry is why
the kind exists as its own entry rather than folding into `obj`.

App and JSM types are classified as `ignored` rather than left to `UNKNOWN`
deliberately. `ignored` in this design means somebody looked and decided;
`UNKNOWN` means nobody has. Leaving a handful of installed-app field types
unclassified would fail the very first run under §9, which is not a useful
failure — it says nothing except that an app is installed.

`ignored` is needed for the **snapshot** side only. Those keys are containers or
derived aggregates rather than field state, and they are present in the issue
JSON; without an explicit kind they would resolve to `UNKNOWN`. On the changelog
side they do not appear at all — with one exception, `comment`, whose items carry
the comment body in `fromString` with every other side NULL. That shape is
byte-identical to a `string_array` cleared to empty, which is a second
independent reason the kind must come from metadata rather than from the item.

Fields that do carry real changelog traffic and must NOT be ignored include
`timespent` and `timeestimate` (`scalar`, and already consumed by
`task_issue_state.time_spent_seconds`), `attachment` and `issuelinks`
(`obj_array`), and `project` (`obj`, an issue moved between projects).

`summary` is an ordinary `scalar` field here, not a special case — see §10 on
retiring the denormalized `title` column.

Anything not matched resolves to `UNKNOWN`, which is a hard failure (§9).

### 3.1.1 Onboarding another Jira instance

Nothing is re-mapped by hand. Three mechanisms already in this design carry the
portability, and the first is the important one:

1. the **round-trip invariant** (§14) validates the separator and shape rules
   against *that instance's own data*, on every run. If the system-versus-custom
   generalization does not hold somewhere, this is what says so, before the
   numbers reach a dashboard. The rule table is a starting point; the test is the
   guarantee.
2. `UNKNOWN` plus the registry means an unrecognised structure never passes
   silently.
3. the operator override table — the same one §3.2 needs for fields missing from
   the catalogue — absorbs a genuinely exotic app field as a row rather than a
   code change and a release.

A one-off report at onboarding time lists the kinds present in the new
catalogue and anything unresolved, so the check is cheap and only asks for
attention when it finds something.

The system-versus-custom generalization above is derived from a single
catalogue. It is stated as the rule because it follows from how Jira renders
changelog entries, but it is the round-trip test, not this document, that
establishes it for any given instance.


### 3.2 Fields absent from the field catalogue

A changelog item may reference a `field_id` that `bronze_jira.jira_fields` does
not contain. Bronze is append-only with ReplacingMergeTree dedup per field, so
the catalogue never forgets a field it has seen once — a field deleted after the
connector's first field sync keeps its row, with `collected_at` simply frozen.
Absence therefore has three distinct causes:

1. the field was deleted from the instance *before* the first field sync — the
   dominant case, and legitimately unclassifiable;
2. the item is not a field at all: Jira emits named pseudo-entries in the
   changelog alongside real fields. These are filtered explicitly in
   `jira__changelog_items`, next to the existing `fieldId != ''` guard, so they
   never reach the classifier;
3. the field was created so recently that its catalogue row has not arrived yet.
   The field list is its own stream inside the same connection, so a field
   created and populated between syncs produces events before metadata.

Cause 3 is the dangerous one: it is indistinguishable from cause 1 by absence
alone, and silently dropping it would reproduce exactly the class of defect this
design exists to remove.

Handling:

- the field's **history is not reconstructed** — no per-event rows;
- **one** synthetic row is emitted per `(issue, retired field)`, carrying the
  last known value taken verbatim from the newest changelog item's `to` side,
  stamped with that item's `event_at`. Values are stored as they arrived, with no
  list parsing, because the field's shape is unknowable;
- that row carries `event_kind = 'unclassified_field'`, not `synthetic_initial`.
  The distinction matters: cardinality and value shape are unknown, so a
  consumer counting "issues where this field was ever set" must be able to tell
  a best-effort value from a derived one. It is also not `retired_field` (§3.6),
  which is the opposite statement — that row asserts the field holds nothing,
  this one carries the last value it is known to have held;
- `field_name` comes from the changelog item's own display name, which is
  present even when the catalogue row is not;
- `jira__task_field_unclassified` lists every excluded field with its item
  count, the issues it touches and its event range, so the exclusion is
  queryable rather than implicit;
- `assert_jira_unclassified_fields_are_old` fails when a field is excluded whose
  changelog activity is *recent* — any event newer than the catalogue's own
  first sync for that source. That instant is written once into
  `jira__catalogue_first_seen` and kept: `bronze_jira.jira_fields` is a
  ReplacingMergeTree versioned by the extraction stamp, so once its parts merge
  `min(_airbyte_extracted_at)` reads as the latest sync, not the first, and a
  guard recomputed from bronze would pass an ever-younger set of events. An
  ancient deleted field can only have old events
  and passes; a newly created field fails immediately. This separates cause 1
  from cause 3 without blocking the run, which is why it is a test and not a
  `throwIf`: the condition is about collection, not about a shape the model
  cannot handle, and the best-effort rows are still there meanwhile.

**This is not a hypothetical branch.** Measured on a real catalogue, tens of
fields with thousands of changelog items were reaching nothing at all, because
the join to the classifier is an inner one — a populated field with no history,
which is the reported defect in a different disguise. Their events all predate
the catalogue's first sync, so the recency guard is green on them; that is the
guard doing its job, not the absence of a problem.

### 3.3 Separators are a property of the kind, not of the value

Jira never parses `fromString` / `toString` back — they are a rendered
representation for the history view, and the authoritative value is the field
itself. There is therefore no separator to detect; there is only a per-kind join
convention to invert:

| kind | separator | why it is unambiguous |
|---|---|---|
| `string_array` | single space | Jira rejects a label containing whitespace, so a space is always a separator and a comma is always content |
| `option_array` | `[`, comma followed by a space, `]` on the id side; bare comma on the display side | ids are numeric, so the bracket form parses without ambiguity; displays are matched positionally to ids |
| `legacy_list` | `", "` | the pre-2020 Sprint serialization |

### 3.4 Id spaces that do not reconcile

Some select fields hold a current value whose option id appears in no changelog
event of that field, while the issues holding the value do have events. The
option set was replaced: every option got a new id, the display text unchanged,
and Jira recorded no event for the change. Within one field the ids in the
issue resource and the ids in the changelog are then two disjoint spaces with
no mapping between them. The same shape occurs on the system `priority` field.

This is documented Atlassian behaviour. Deleting and recreating a custom field
context generates a new set of option ids that do not match the old ones, and
Atlassian calls that data loss; deleting an option discards its historical
information, and the supported way to retire one is to **disable** it, which
keeps history intact. The recovery paths Atlassian documents — bulk reassignment
via JQL, or remapping ids in the database — apply before the fact, not after.

Nothing here can repair it: the old-to-new mapping does not exist in the data.
The pipeline can only **detect** it and refrain from presenting such ids as
comparable over time. Consequences, all real:

- a metric grouping history by option id splits one value in two at the
  boundary;
- grouping by display text merges them, but is wrong for the issues whose value
  actually changed during the replacement;
- reconstructed initial state splits inside one field — an issue with events is
  rolled back into the old id space, while an issue with no events keeps the
  resource's new id.

Detection is a test, not a mechanism:
`assert_jira_field_id_spaces_intersect` reports a field whose changelog id set
and current-value id set are disjoint while its event coverage is high. Two
guards keep it honest, and both were needed:

- **scope.** Only the kinds whose id names a value from an administered,
  reusable set — `option`, `option_array` and the system objects. For a `scalar`
  or a `datetime` the id IS the value, so a field whose values have all moved on
  looks disjoint and is not; attachment ids are per issue and never repeat;
  sprint ids only grow.
- **coverage.** Only fields whose events reach a large enough share of the
  issues holding a value. A recently added field has values and almost no
  history, which is a young field, not a broken space. Coverage is the share of
  value-holding issues that also have an event — not a ratio of two independent
  counts, which can exceed one and makes the threshold meaningless.

**The replacement can be whole or partial, and only the whole one is a field-level
condition.** If the field kept being used afterwards, the old and new spaces
coexist: the field's own sets intersect, and the test correctly stays quiet.
What is then observable is per pair — an issue whose last event predates the
replacement has a newest state naming an id that appears in no current value of
that field anywhere. Measured, the partial case is by far the larger of the two,
and the system `priority` field is one of them. Reporting it per pair would
report an unfixable source condition on thousands of rows, so the test covers
the field-level case and this paragraph records the other.

**Recommendation to the Jira administrators, independent of this work:** disable
options instead of deleting them, and avoid deleting and recreating field
contexts. Nothing downstream can recover history once the option ids are gone.

### 3.5 Zero is not a value, for a time-tracking estimate

The round trip assumes the two sides can express the same set of states. For
`timeestimate` and `timeoriginalestimate` neither side can express the one
distinction that would matter.

What was measured:

- the issue resource reports "no remaining estimate" as `0` or as `null`, and
  which one it picks tracks exactly whether the `timetracking` container carries
  `remainingEstimateSeconds` — a rendering property, not a fact about the
  estimate;
- the changelog writes the literal `0` for both, and the items are otherwise
  byte-identical: same `from`, same `to`, same rendered sides;
- the dominant event is `null → 0`, which is what logging work against an
  unestimated issue emits. It cannot be a deliberate zero: the field was never
  set;
- an estimate genuinely worked down (`28800 → 0`) also comes back from the
  resource as `null`, so the resource's `null` covers "cleared to zero" as well
  as "never recorded";
- and the resource's `0` covers both "nothing recorded" and "estimated then
  consumed" — the question of whether an issue was ever estimated is answered by
  `timeoriginalestimate`, a different field, not by this one's zero-versus-null.

So there is no state that `0` denotes and absence does not. Both spellings fold
to the empty state, on both sides, under the `duration` kind. This reconciles
the pairs rather than exempting them, and it is a value-domain rule for two
named Jira **system** fields — the same class of rule as "a label cannot contain
whitespace", and the reason the classifier's invariant bans instance-specific
`customfield_` literals rather than all literals.

What is given up: an operator who deliberately typed a zero cannot be
distinguished from one who typed nothing. That was already true of the
changelog, and true of the resource whenever the time-tracking container is
unpopulated, so no recoverable information is lost.

The rule is deliberately narrow. A story-point estimate is the same structure —
a plain number — and a zero there is a value somebody entered, so it stays
`scalar`. That is why the kind is matched by field name and not by the value
being numeric; a test pins both halves.

### 3.5.1 A date or an instant moved, unlogged

The same family as §3.4 and §3.5, reached a third way, and recorded because the
shape of the evidence is what rules out the tempting explanations.

For the date-picker fields, the value the issue holds and the value the field's
newest changelog item recorded differ by **0 or +1 day and by nothing else** —
never −1, never two. So it is not "the date was changed again without an event",
which would scatter; and it does not correlate with the event's time of day, so
it is not a rendering that crosses midnight in one timezone and not the other.
Both sides of the item agree with each other, so it is not a choice between the
id side and the display side either.

What remains is a bulk or automated postponement by one day that Jira did not
journal. Nothing in the data says which issues it touched, so nothing here can
repair it. It is small, and the round trip is what surfaces it.

The same shape appears on `datetime` fields as a fixed offset of a couple of
hours, on a related pair of fields at once — a planned window shifted as a
whole. Both sides of those items carry an explicit UTC offset, so it is not a
rendering or a timezone assumption: the values genuinely differ and no event
records the change.

**Residue.** With this and the other named conditions set aside, the round trip's
remaining failures are all source-side: a replaced id space (§3.4), a value
domain the log cannot express (§3.5), an unlogged edit (here), an element
removed without an event, a link to an entity that does not live in the field
(§3.7), and an app field whose changelog entries carry no content (§3.5.2).
Nothing left in it is a fault of these models — which is the claim the test
exists to be able to make.

### 3.5.2 Two more source-side gaps, named for completeness

Neither is repairable and both are small, but naming them is what stops the next
reader from hunting for a parsing defect.

**An element removed without an event.** For the element-wise kinds a version, a
component or an attachment sometimes disappears from the issue while the
changelog records no removal, so the journal's newest state holds one element
too many. Same shape as §3.4 and §3.5 — the source changed state silently — on a
different kind.

**An app field whose changelog entries carry no content.** A read-only field
computed by an app (a checklist progress counter, for example) emits changelog
items whose id sides *and* string sides are all empty. These are the degenerate
items §6 skips, so the pipeline emits nothing from them, which is correct: there
is no state in them to record. The consequence is that such a field has a
current value and no reconstructable history.

Those fields are deliberately **not** classified `ignored`. `ignored` would drop
their current value too, and the value is real — a metric can use it. What is
missing is history, and no classification recovers that.

### 3.6 Withdrawal of a field from an issue

A field stops being returned for an issue when the project's or the issue
type's field configuration changes, or when the field is deleted from the
instance. Jira emits no changelog item for it — the key simply stops appearing.
Without an event the journal's newest state stays at whatever the field last
held, which is a value the issue does not have.

One synthetic row per `(issue, withdrawn field)` records it:
`event_kind = 'retired_field'`, empty value arrays, `event_id = 'retired:{issue_id}'`,
stamped with the moment the absence was observed — the issue's own bronze
extraction mark, which is the same stamp the round trip uses as the issue's
freshness, so the event is never newer than the state it is compared against.
This is how issue availability is already modelled (§11): disappearance is an
event, dated by detection, because the source exposes no other date.

The cause is deliberately not classified. "Deleted from the instance" and
"removed from this issue's context" are the same observation from here, and
telling them apart would need the field catalogue's last-seen mark to agree with
the issue's — two streams read at different points of one sync.

**Only an absent key qualifies.** A key present with an empty value means the
field still applies to the issue and is unset (§6), which is an ordinary state.
If the journal disagrees with that, a clearing event is genuinely missing, and
that must surface as a round-trip failure rather than be overwritten by a
synthetic row — masking it would blind the invariant to the very defect class
this design exists to remove.

Note that this is a **narrower** mechanism than the failure class it was
expected to close. A journal state that the resource does not confirm has
several distinct causes, and an absent key is only one of them; §3.5 is another,
and the round trip's own inability to tell "the snapshot has no row" from "the
value is empty" is a third. Each needs its own treatment, and one synthetic
event cannot stand in for the others.

### 3.7 Issue links: the two sides name different things

An issue link is element-wise in the changelog, like a component — one item per
link added or removed. It is not an `obj_array`, because the two sides identify
the element differently:

- the **issue resource** holds the LINK OBJECT: `{"id": <the link's own id>,
  "outwardIssue" | "inwardIssue": {"key": ...}, "type": {...}}`. A
  component-shaped normalizer takes the top-level `id`, which is the link's own;
- the **changelog** names the LINKED ISSUE by key in the id side, with a
  rendered sentence in the display side ("This issue duplicates PROJ-1"). The
  link's own id appears nowhere in it.

So identifying by the link id leaves the two sides with no id in common, and
every issue holding a link disagrees with its own history. Measured, this was
the single largest remaining round-trip class after the metadata-driven parsing
landed — and it looked like a fold defect until the two shapes were put side by
side.

The key is the identifier on both sides, and the link id is discarded; it stays
in the issue JSON. Direction is not part of the identity: an issue holds each
link once, and which side of it the issue is on is carried by the rendered text.

Two consequences worth stating:

- **two links to the same issue with different types collapse into one
  element.** The changelog gives them the same id, so the pipeline cannot hold
  them apart, and a display would be the only difference — which the contract
  does not treat as identity.
- **a remote link is logged in the same field.** A link to a Confluence page
  emits an item whose id side is the remote link's numeric id, and no such
  element exists in `issuelinks` at all — remote links are a separate REST
  resource. Those rows carry an id the current value can never confirm. They are
  a small fraction of the field's items and are left as they arrive rather than
  detected by content.

## 4. Value normalization

Value normalization lives in `dbt/macros/jira/jira_field_value.sql`. Every macro
returns a `Tuple(Array(String), Array(String))` expression, so a caller reads
`.1` for ids and `.2` for displays; `jira_norm_value(kind, raw_json)` dispatches
on `field_kind`. This is the layer the current pipeline lacks — today the
snapshot model hand-writes one JSON path per field, which is why only about ten
fields are covered.

There are eight normalizers, not one per kind, because the kinds that differ in
*delta application* often agree on how a value is *read*:

| macro | kinds | notes |
|---|---|---|
| `jira_norm_scalar` | `scalar` | id and display are the same text |
| `jira_norm_datetime` | `datetime` | the same, reduced to one canonical instant (§3.1) |
| `jira_norm_duration` | `duration` | the same, with zero folded to the empty state (§3.5) |
| `jira_norm_link_array` | `link_array` | the linked issue's key as both value and identifier (§3.7) |
| `jira_norm_single_obj` | `obj`, `option`, `user` | one `(id, display)` pair |
| `jira_norm_issue_ref` | `issue_ref` | display only; the id spaces do not reconcile |
| `jira_norm_string_array` | `string_array` | bare strings, ids equal displays |
| `jira_norm_obj_array` | `obj_array`, `option_array`, `legacy_list` | array of objects |

Two properties of the real data drive the shape of these macros, and both were
found by measuring rather than reasoning:

**Ids are not consistently quoted.** An option id arrives as `"19272"` and a
sprint id as `2151`, in the same structural position. Every id and scalar
therefore passes through `jira_json_unquote`, which unquotes a JSON string and
passes a bare number or boolean through unchanged. `JSONExtractString` alone
returns an empty string for an unquoted number, so using it would silently drop
every sprint id.

**One kind can carry several element spellings.** `option_array` covers
`schema_items` of `option` (`{id,value}`), `user` (`{accountId,displayName}`)
and `version` (`{id,name}`). Branching per kind would lose the display for two
of the three, so element extraction coalesces over the spellings instead:

- id: `id`, then `accountId`;
- display: `value`, then `name`, then `displayName`, then `key`.

The display probe order is load-bearing. A `project` object carries **both**
`name` and `key`, and the changelog renders its name, so `name` must be probed
before `key` for the two sides to reconcile. A `parent` object carries `key` and
no `name`, which is why `key` stays in the list at all.

## 5. Delta application

Per kind, one macro that reverse-applies an event to a state, and one that
forward-applies it. Both operate on the normalized pair.

| kind | reverse (state before event) | forward (state after event) |
|---|---|---|
| `scalar`, `datetime`, `duration`, `option`, `user`, `obj`, `long_text` | the event's `from` side | the event's `to` side |
| `string_array`, `option_array`, `legacy_list` | the event's parsed `from` list | the event's parsed `to` list |
| `obj_array`, `link_array` | element added → drop it; element removed → put it back | the mirror |

Only the element-wise kinds read the incoming state. Every other kind ignores it, which is
why those kinds cannot accumulate error and why a broken seed cannot corrupt
them.

**An element is identified by its id, never by its rendered pair.** The state
arithmetic carries each element as `id \x1f display` so the two arrays cannot
drift apart, and the set difference must still key on the id alone. A component
or a version renamed after the event that touched it arrives with one display in
the changelog and another in the issue JSON: the same element, two different
pair strings. Differencing the pairs then fails to undo the addition, and the
reconstructed initial state claims the issue was created carrying something
added later.

The round trip does not catch this — it compares ids, and the id set is correct
either way. Only the initial row is wrong, which is why it took a test over
controlled inputs to surface. Found that way, not by reading the code.

**Two events of the same millisecond are ordered by the numeric changelog id.**
Jira's changelog id is monotonic, so it is the right tie-break, but it reaches
staging as a String — and as text `'101'` sorts before `'99'`, which inverts a
pair of events every time the id crosses a digit-count boundary. For an
element-wise field an inverted add/remove pair changes the resulting set, so the
comparison is numeric wherever it orders and stays a string wherever it
identifies (the event id and the unique key).

Two properties of the changelog constrain the handlers, both found by measuring:

**A scalar field's id side is not reliably present.** Duration fields carry the
same text in the id and display sides; a date carries the machine value in the id
side and a rendered one in the display side; a summary, a description and a
story-point estimate carry nothing in the id side at all. The handler takes the
id side when present and the display side otherwise — for a date that is also the
side which reconciles with the issue JSON.

**A snapshot-shaped field can emit display-only items.** Not every field of the
bracketed-id family supplies ids: an app field can send items whose id side is
empty and whose display holds the value. Falling back to the (empty) id side
loses the event outright — the same failure mode this design exists to remove —
so when the id side is empty the displays serve as both value and identifier,
exactly as they do for a labels-type field.

An item whose four sides are **all** empty is the single degenerate case and is
skipped, for every kind rather than only for the element-wise ones. Clearing a
field to empty is not degenerate: the `from` side still names what was removed,
and a field explicitly set to an empty list is an ordinary event whose resulting
state is empty.

Normalization is implemented for **every** kind except `long_text`, which is
**in scope for this change set** and not yet written: it needs the side table of
§8 first. Until that lands a long-text field carries no state row, which is a
sequencing decision inside one change, not a permanent gap — the change is not
complete while the largest single family of changelog traffic after the scalar
kinds still produces nothing.

Delta application is staged: `Set` for the single-value kinds, `Add`/`Remove`
for `obj_array`, and the full-list `Snapshot` for `string_array`. The remaining
snapshot-shaped kinds (`option_array`, `legacy_list`) reuse the same
`Snapshot` handler once their changelog parsing lands, and raise until then.

Scoping normalization narrower than this was considered and rejected: with
`option_array` alone covering scores of fields that hold values on real issues,
a handler that raises on them would fail the first run — a failure that carries
no information.

## 6. Absent, empty and degenerate values

Three states are distinguishable in the issue JSON and must stay distinguishable:

| in `custom_fields_json` | meaning | emitted |
|---|---|---|
| key absent | the field is not in this project's or issue type's field context | no row |
| key present, `null` / `[]` | applicable, not filled | row with empty value arrays |
| key present, value | the value | row with the value |

The current snapshot model collapses the first two, because it emits a row with
empty arrays for every hardcoded field regardless of whether the issue has it.
For quality metrics the difference between "not configured" and "configured but
empty" is the difference between nothing to measure and a gap in the process.

Clearing a multi-value field to empty is a **normal** operation, not an
exception: it arrives as `to` NULL on an element-wise field, as an empty right
side on a snapshot-shaped field, and both must be honoured. The genuinely
degenerate case is an item where the id sides *and* the string sides are all
NULL, which carries no information; those are skipped.

## 7. Incremental strategy

No high-water mark. The unit of recomputation is the `(issue, field)` pair.

1. Find pairs whose changelog set in bronze differs from what the output table
   already records — a new `changelog_id` for that pair, or a changed issue
   snapshot.
2. Recompute those pairs **in full**, from the current value backwards.
3. Replace by `delete+insert` keyed on the pair.

This is idempotent, removes the seam entirely, and makes `--full-refresh` work
by ordinary dbt semantics. It also retires
`reset_task_field_history_on_full_refresh` and the `CREATE TABLE IF NOT EXISTS`
macro that forced it to exist.

The seam check survives as a **test**, not as a mechanism: recomputing a pair
must reproduce the rows it previously held, for every event at or below the
issue snapshot's `collected_at`. Events newer than the snapshot are excluded
from the comparison — the issue stream and the history stream are read at
different points within a sync, so an event newer than the snapshot is expected
and self-heals on the next run.

Idempotence relies on `unique_key` being a pure function of content. The
existing convention already satisfies this:

    unique_key = {insight_source_id}-{data_source}-{id_readable}-{field_id}-{event_id}

where `event_id` is the bronze `changelog_id` for changelog rows and
`initial:{issue_id}` for synthetic rows. Two runs over the same bronze data
therefore produce byte-identical keys, and ReplacingMergeTree collapses them.

## 8. Long text in a side table

Status: implemented (`jira__task_field_text`). §5's normalizers
cover every other kind; this is the remaining one.

`long_text` values are large, change often, and are stored per event, so a naive
layout copies the whole body into every history row.

A second model, `jira__task_field_text`, holds `(text_id, content)` with
`text_id = sipHash128(content)`; ReplacingMergeTree on `text_id` deduplicates
identical bodies for free. The history row stores `value_displays =
[hex(text_id)]` and `value_id_type = 'text_ref'`.

One dbt model writes one table, so this is simply a second model over the same
upstream — no extra orchestration step.

This makes long text cheap to store. It does not make it accurate: `toString`
for a long-text field is Jira's rendering and may be truncated, so a
reconstructed historical body is "as Jira rendered it", not the source text.
Consumers needing exact bodies must read the issue snapshot, not the history.

## 9. Failure policy

Unknown or unhandled shapes fail the run loudly. No silent skips, no anomaly
column.

    throwIf(field_kind = 'UNKNOWN',
            'unmapped Jira field kind — query staging.jira__task_field_kind')

ClickHouse requires `throwIf`'s message to be a **constant**, so it cannot name
the offending field. The message points at the classifier view instead, which is
queryable, and `assert_jira_field_kind_covers_catalogue` lists the rows.

Every handler ends with the same guard on its own invalid inputs. The accepted
consequence is that one unmapped field stops the whole nightly Jira pipeline,
silver and metrics included, until someone intervenes. This is deliberate: the
defects this design replaces were both silent.

The first exception is a field absent from the catalogue (§3.2), which cannot be
classified even in principle — the run must not stop for it, because no
intervention on this side can resolve it. Those are excluded from reconstruction, recorded in
the registry, and covered by the recency test — so the exclusion is visible and
bounded rather than silent.

The second is a state the source changed without recording it (§3.4, §3.5).
There the run does not fail, because no shape is unmapped and nothing the
pipeline does is wrong; a test names the condition instead, and the affected
pairs are excluded from the round trip by a stated rule rather than by a
tolerance.

## 10. Contracts that must not change

The output table is consumed by `silver.class_task_field_history` through
`union_by_tag`, and a YouTrack twin is expected to emit the same shape.

- the per-issue creation marker: one row with `field_id = 'created'`,
  `event_kind = 'synthetic_initial'`, `_seq = 0`, carrying the reporter and the
  creation timestamp, and no value. `task_issue_current_state.created_at` reads
  it as `minIf(event_at, event_kind = 'synthetic_initial')`
- `event_id`: `initial:{issue_id}` for synthetic rows, the changelog id for
  changelog rows
- `_seq`: 0 for changelog rows; for `synthetic_initial` rows, the 0-based index
  of the field in the `field_id`-ascending list. **`(event_at, _seq)` is not a
  total order**, and the claim that it is was wrong in two ways:

  - two changelog rows of one millisecond both carry 0. The tie-break is the
    event id, compared numerically — it is the changelog id, and Jira's is
    monotonic;
  - worse, `_seq` sorts an initial row *after* a changelog row of the same
    instant, because the initial rows carry 1..N and the changelog rows 0. Every
    initial row of an issue is stamped with the creation timestamp, so this
    happens to every issue whose first event landed on its own creation — and
    the newest state of that field then reads as the empty state it had before
    the event. The kind is what orders them (`jira_event_rank`): an initial row
    is by definition the state before any event, and a `retired_field` row is
    stamped at or after every event.

  Found by measuring, not by reading: it accounted for the entire round-trip
  failure of one epic-link-shaped field and part of several others.

  This also reaches gold. `task_issue_state` reads current state as
  `argMax(..., (event_at, _version))`, and `_version` is a build-time stamp
  shared by every row of one build — so on a same-instant pair it ties too, and
  resolves arbitrarily. Widening `_seq` to carry the changelog id would make the
  ordering intrinsic and is the better fix, but it is a contract change across
  every source, so it is recorded here rather than made
- the `event_kind`, `delta_action`, `field_cardinality` and `value_id_type`
  enums, and the `value_ids` / `value_displays` array pairing. `event_kind` gains
  two values, `retired_field` (§3.6) and `unclassified_field` (§3.2); the enum is
  spelled out in a `CAST` in each contributing model, so all arms of the union
  change together. `value_id_type` is unchanged by the new `datetime` kind: like
  `scalar` it reports `none`, so no field's identifier type moves at cutover
- `unique_key` as the single ORDER BY column

### 10.1 Retiring four columns, and why the fifth stays

`author_display`, `delta_value_id` and `delta_value_display` leave
`silver.class_task_field_history`. None of them has a reader in gold, in silver
or in the backend — checked, not assumed —
and the class contract's job is to serve ready state: a consumer that needs the
detail of one change joins back to the event it came from, a path
`assert_changelog_traceable_to_bronze` guarantees. The two lifecycle arms carried
their entity id in `delta_value_id` **and** in `value_ids[1]`, so nothing is
lost there either.

**`title` stays until the cutover**, and the plan to drop it with the others was
wrong for a reason worth stating: the title's *producer* changes at cutover, not
before.

Gold reads the title through the `title` role, and for Jira that role binds
`summary`. While the Rust binary is still the producer, a `summary` row exists
only where `summary` reaches its snapshot or its changelog — and the snapshot
model it reads does not list `summary`, so the row exists only for an issue
whose summary was actually changed. An issue never renamed would therefore have
no title at all, while today it has one: the binary fills the `title` COLUMN for
every row it writes, from the issue's own summary.

So the column and the role have to coexist for one release. Gold reads the role
first and falls back to the column, which is what makes the GitHub half work
immediately and keeps the Jira half working until its own producer changes. Both
the column and the fallback go with the binary.

This was found by replaying the binary locally (§15) and comparing per issue —
not by reading either implementation. The spec asserted the opposite ("the Jira
side never populated it at all"), which was simply false.

**`delta_action` stays** for a related reason. It has three real readers — the ten role predicates in
`task_issue_state`, `assert_bound_values_are_mapped`, and
`assert_delta_action_matches_cardinality` — and, more importantly, it is the
only thing that distinguishes a comment being added from one being edited or
removed. Those rows are not state rows: `value_ids[1]` is an entity id, not a
value, so the journal's other columns say nothing about what happened to it.

Dropping it therefore needs a replacement discriminator, and the honest one is
to widen `event_kind` so the action is part of the kind for a sub-entity
(`lifecycle_add` / `lifecycle_set` / `lifecycle_remove`). That is a cross-source
enum change with its own migration and its own consumers to check, so it is
recorded here rather than folded into this one.

### 10.1.1 The `title` column: how the role takes over

The title is an ordinary field, so it moves into the journal as one and binds to
a role; the column stays for the reason §10.1 gives, and this is the order of
the pieces:

1. **Jira**: the new model emits `summary` like any other `scalar` field — a
   `synthetic_initial` row from the issue JSON plus its changelog events. Jira
   gains rename history it does not have today.
2. **GitHub**: `github__task_field_history` adds a union arm emitting
   `field_id = 'title'`, sourced from the issue-title CTE it already joins, and
   keeps filling the column from the same CTE.
3. **Role binding**: `summary` and `title` bind to a `title` role in
   `config.task_field_roles`. Gold reads roles, never vendor field ids — the
   invariant is stated in `github__task_field_history` itself.
4. **Gold**: `task_issue_state` reads the role first and falls back to the
   column:
   `coalesce(nullIf(argMaxIf(value_displays[1], (event_at, _version), role = 'title'), ''), argMax(title, (event_at, _version)))`.
   `task_metric_evidence` needs no change: it reads `title` from
   `task_issue_state`, not from the journal.
   `tests/jira/transform/test_title_role.py` holds the precedence in place.

Dropping `title` from the staging arms and from `silver.class_task_field_history`
is a migration under `src/ingestion/scripts/migrations/` in the cutover change,
once the derived model has produced a `summary` row for every issue; the
`ADD COLUMN IF NOT EXISTS ... AFTER id_readable` self-migration in the DDL macro
goes with the macro itself.

Note that the "Rust owns this table" decision is referenced in code comments as
ADR-003 but has no ADR file in the repository. Retiring the binary should record
the reversal wherever that decision ends up living.

## 11. Issue deletion

The changelog has no deletion event. Disappearance is detected through
`bronze_jira.jira_issue_census` and already modelled by the
`jira__issue_availability_*` chain. This design consumes that signal rather than
inventing one, and adds no deletion handling of its own.

## 12. Dropping the duplicated JSON column

`bronze_jira.jira_issue` stores the same payload twice: `fields` (the raw
property, serialized by the destination, keys in API order) and
`custom_fields_json` (the same object re-serialized by the connector's
`| tojson`, keys sorted). The key sets are identical and every value difference
is nested-key reordering, so no information is lost by keeping one.

Keep `custom_fields_json`: it has readers, its key order is deterministic, and
fixtures compare against it. Remove `fields`, which has none.

Order matters — dropping the column while the connector still emits it lets the
destination recreate it:

1. add a `RemoveFields` transformation for `fields` to the `jira_issue` stream
2. bump the connector descriptor, deploy, let one sync complete
3. add a migration under `src/ingestion/scripts/migrations/` performing
   `ALTER TABLE bronze_jira.jira_issue DROP COLUMN IF EXISTS fields`
4. drop the column from `src/ingestion/scripts/connectors-ddl/jira.sql`

This is storage hygiene, independent of the defects above, and can land
separately.

## 13. Memory

Two models read a JSON column that is gigabytes wide, and both must resolve
their dedup to a raw id FIRST, in an aggregation that carries only that String,
then fetch the payload by joining back:

- `jira__issue_field_snapshot` reads `custom_fields_json`;
- `jira__changelog_items` reads `items`. Its earlier shape — `SELECT *` ordered
  by the extraction stamp with `LIMIT 1 BY` — put every row's payload into the
  sort buffer. On a sample the two-pass form more than halved peak memory and
  ran twice as fast, for byte-identical output.

The model that reads `custom_fields_json` must inherit the argMax pattern and
the spill settings introduced for the current snapshot model. Extraction has to
happen block-wise inside the aggregate so that only the small extracted tuple
survives to the next stage; a sort or a buffer that carries whole JSON payloads
exhausts the server. Concretely, the key/value unpivot of the issue JSON must
sit **after** the argMax dedup, never before it.

## 14. Tests

Three layers.

**Unit, per macro.** One case per kind per direction: normalize a JSON value,
reverse-apply an event, forward-apply an event. Each macro is independently
callable, so each gets a table of inputs and expected pairs, including the
degenerate inputs it must reject.

These are written as dbt **singular tests over literal fixtures**, not as
`unit_tests:` blocks: the fixture rows are an inline `arrayJoin` of tuples, the
macro is invoked on them, and the test selects the rows where actual differs
from expected. That touches no table, so it is a true unit test, and it needs no
machinery the repository does not already have — the project has no
`unit_tests:` block anywhere, and the `dbt` on a developer machine may be a
Fusion preview that cannot validate against dbt-clickhouse.

**Portability, over the source text.** The classifier must contain no
`customfield_` literal (§3.1). Field ids are instance-specific and type
constants are not, so this single grep is what keeps the rules portable; it is
cheap and it fails loudly the first time someone reaches for a field id.

**Invariant, in SQL, over real data.** The oracle the pipeline has never had:

- *round-trip*: the newest reconstructed state of a `(issue, field)` pair must
  equal the value in `custom_fields_json`, parsed by that field's kind. This
  validates the separator rule for every kind automatically, on every run, and
  would have caught both defects in §1.
- *bronze coverage*: every field key present with a non-null value in an issue's
  JSON must have at least one row for that issue in the history. The existing
  tests all run history → bronze; this is the missing direction.
- *seam*: recomputation reproduces previously recorded rows, restricted to
  events at or below the snapshot's `collected_at` (§7).
- *retired-field recency*: no field is excluded for catalogue absence while
  carrying an event newer than the oldest `collected_at` in the catalogue
  (§3.2) — this is the guard that distinguishes a deleted field from one whose
  metadata has not arrived yet.
- *disjoint id spaces*: no field's changelog id set and resource id set are
  disjoint while its event coverage is high (§3.4). This cannot be repaired, so
  the test exists to name it before a metric is bound to such a field.
- *withdrawal covers what it claims*: every `(issue, field)` pair the round trip
  reports must fall into one of the causes §3.4-§3.6 enumerate. A pair that
  matches none of them is a defect in this model, and lumping it in with the
  irreconcilable ones is how the previous shape of this document mis-scoped the
  work.
- the existing singular tests on ordering, cardinality and event-id conventions
  are retained.

**End-to-end.** Two layers, and the split is deliberate.

The shapes below are covered by the **transformation lane**
(`dbt/tests/jira/transform/`): each seeds the three bronze inputs, builds the
Jira staging chain with real dbt against a real ClickHouse, and asserts the
whole journal for the issue. That is where a parsing or reconstruction defect is
visible, and it is strictly stronger than a metric assertion for these shapes.

Their **metrics-layer** counterparts in `tests/datapath/metrics/tasks/` are deliberately
NOT written yet, for two reasons that both dissolve at cutover:

- the journal that reaches silver today is the Rust binary's, not this model's,
  so a case seeded now would assert the behaviour being replaced;
- the YAML rig asserts the analytics HTTP response, and its expect engine binds
  metric-shaped payloads. None of these shapes reaches a metric — there is no
  labels metric, no components metric — so a case could assert the request
  succeeded and nothing more.

The one part of this change whose consumer IS gold — the `title` role — is
covered now, in the transformation lane, because it can be: `test_title_role`
seeds the class table and builds `task_issue_state`, pinning the precedence
between the role and the column it falls back to (§10.1).

Shapes covered, one test each:

- a labels-type field changed several times, never present in any snapshot list
- an element-wise multi-value field with interleaved adds and removes
- a bracketed-id multi-select field
- a field set at creation and never changed
- a field absent from the issue's field context entirely — asserting **no** row
- a multi-value field cleared to empty
- a `comment` item, asserting it produces no field-state row despite looking
  exactly like a cleared `string_array`
- a changelog field absent from the catalogue, asserting exactly one
  `unclassified_field` row carrying the newest value at the newest event's
  timestamp
- a field whose key leaves the issue JSON while its history holds a value,
  asserting exactly one `retired_field` row with empty value arrays, and that
  the round trip then passes for that pair (§3.6)
- the same with the key present and empty, asserting **no** `retired_field` row
  and a round-trip failure — the case the withdrawal event must not absorb
- a datetime field set once, asserting the changelog spelling and the resource
  spelling of one instant produce one value (§3.1)
- an issue whose summary changed, asserting the title reaches gold through the
  `title` role rather than a denormalized column

## 15. Comparing against the binary it replaces

The comparison does not need the cluster. The binary reads staging and bronze
and writes one table, so a restored bronze dump plus a locally built
`jira-enrich --features io` reproduces its output next to the model's, on
**identical** inputs, and the diff is a pure implementation difference.

Two traps make the naive version meaningless, and both were hit:

- **the binary hardcodes `bronze_jira`.** Building the dbt chain from a sampled
  copy while the binary reads the full database compares two different
  datasets — the row counts differ by an order of magnitude and nothing about
  the diff means anything. Rename the sample to `bronze_jira` for the run;
  renaming a database back is cheap and reversible.
- **the intended differences are the bulk of the diff.** A canonicalized
  datetime, a folded zero, an issue key instead of a numeric id, a deduplicated
  bracketed list — these are the change, not defects, so each has to be named
  and set aside before the residue is worth reading.

What the diff establishes, once both sides run on one slice:

- every row the model emits and the binary does not is accounted for by a fix:
  the discarded labels-type events and their initial state, the fields absent
  from the catalogue (§3.2), and the withdrawal events (§3.6). Nothing is
  emitted that cannot be traced to one of them;
- every row the binary emits and the model does not carries **no value at all**,
  except on fields the classifier marks `ignored` — chiefly `comment`, whose
  items are not field state (§3.1) and whose history belongs to
  `class_task_comments`. A handful of `team` and `securitylevel` values are also
  dropped by that decision; they are listed here so an operator can revisit the
  decision rather than discover it;
- where the two disagree on a shared row, the model is right and the binary is
  not. The clearest case: for a labels field the binary discards every event and
  emits a single `synthetic_initial` holding the value from the last event it
  ignored, dated neither at creation nor now. The model emits the initial state
  and one row per event.

That last point is the one worth keeping. The binary's output looks plausible
row by row — a field, a value, a timestamp — and is wrong in a way only a
side-by-side replay exposes.
