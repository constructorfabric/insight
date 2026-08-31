use sea_orm::{DatabaseConnection, FromQueryResult, Statement, Value};
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::api::error::MetricError;
use crate::domain::metric_definitions::definition::{AliasCollapse, MetricInputRole};
use crate::domain::metric_definitions::{
    ComputationSpec, EvidenceGranularity, MetricDefinition, StoredPresentation,
    load_definitions_with_ids,
};
use crate::domain::metric_results::{normalize_key, normalize_metric_key};

use super::capability::EvidenceInputRow;
use super::capability::healthy_evidence;
use super::cursor::{
    decode_cursor, evidence_snapshot_id, selection_fingerprint, verify_evidence_snapshot,
};
use super::dto::{
    DEFAULT_PAGE_LIMIT, EvidenceInput, EvidencePlan, MAX_DISPLAY_DIMENSIONS, MAX_ENTITY_PERSONS,
    MAX_EXPORT_ROWS, MAX_FILTER_VALUE_BYTES, MAX_FILTER_VALUES, MAX_FILTERS, MAX_PAGE_LIMIT,
    MAX_PERIOD_DAYS, MAX_SEARCH_BYTES, MetricDrilldownColumn, MetricDrilldownColumnType,
    MetricDrilldownEntity, MetricDrilldownExportRequest, MetricDrilldownFilter,
    MetricDrilldownPeriod, MetricDrilldownRequest, MetricDrilldownSelection,
    ValidatedMetricDrilldown,
};
use super::presentation::presentation_columns;
use super::sort::{MetricDrilldownSort, column_sql};

struct CommonRequest {
    metric_key: String,
    entity: MetricDrilldownEntity,
    period: MetricDrilldownPeriod,
    filters: Vec<MetricDrilldownFilter>,
    display_dimensions: Vec<String>,
    sort: Option<MetricDrilldownSort>,
    search: Option<String>,
    limit: usize,
    max_limit: usize,
    cursor: Option<String>,
}

use super::error::{
    config_error, db_error, evidence_unavailable, invalid, invalid_error, parse_date,
};

pub async fn validate_request(
    db: &DatabaseConnection,
    ch: &insight_clickhouse::Client,
    tenant_id: Uuid,
    req: MetricDrilldownRequest,
) -> Result<ValidatedMetricDrilldown, CanonicalError> {
    validate_common(
        db,
        ch,
        tenant_id,
        CommonRequest {
            metric_key: req.metric_key,
            entity: req.entity,
            period: req.period,
            filters: req.filters,
            display_dimensions: req.display_dimensions,
            sort: req.sort,
            search: req.search,
            limit: req.limit.unwrap_or(DEFAULT_PAGE_LIMIT),
            max_limit: MAX_PAGE_LIMIT,
            cursor: req.cursor,
        },
    )
    .await
}

pub async fn validate_export_request(
    db: &DatabaseConnection,
    ch: &insight_clickhouse::Client,
    tenant_id: Uuid,
    req: &MetricDrilldownExportRequest,
    limit: usize,
) -> Result<ValidatedMetricDrilldown, CanonicalError> {
    validate_common(
        db,
        ch,
        tenant_id,
        CommonRequest {
            metric_key: req.metric_key.clone(),
            entity: req.entity.clone(),
            period: req.period.clone(),
            filters: req.filters.clone(),
            display_dimensions: req.display_dimensions.clone(),
            sort: req.sort.clone(),
            search: req.search.clone(),
            limit,
            max_limit: MAX_EXPORT_ROWS + 1,
            cursor: None,
        },
    )
    .await
}

// Canonical person id, like every other person-keyed route since the identity
// cutover; the pre-cutover email shape (and the nil UUID, which parses but is
// never a person) is a loud 400.
//
// INVARIANT: this runs before the visibility gate on both drilldown handlers,
// so it must not reach any backend — an unauthorized caller gets no further.
pub(crate) fn parse_person_entity(
    entity: &MetricDrilldownEntity,
) -> Result<(String, Uuid), CanonicalError> {
    let Some(id) = entity.person_id() else {
        return Err(invalid_error("entity.type", "entity must select a person"));
    };

    let person_id = Uuid::parse_str(id.trim())
        .ok()
        .filter(|id| !id.is_nil())
        .ok_or_else(|| invalid_error("entity.id", "entity.id must be a person UUID"))?;

    Ok(("person".to_owned(), person_id))
}

/// Every person a selection reads, canonical and deduplicated.
///
/// INVARIANT: like `parse_person_entity`, this runs BEFORE the visibility gate
/// on both handlers, so it reaches no backend — an unauthorized caller gets no
/// further than the shape of their own request.
pub(crate) fn parse_person_ids(
    entity: &MetricDrilldownEntity,
) -> Result<Vec<Uuid>, CanonicalError> {
    let ids = entity.person_ids();
    if ids.is_empty() {
        return Err(invalid_error("entity.ids", "entity must select a person"));
    }
    if ids.len() > MAX_ENTITY_PERSONS {
        return Err(invalid_error(
            "entity.ids",
            format!("entity.ids must name at most {MAX_ENTITY_PERSONS} people"),
        ));
    }
    let mut parsed = ids
        .iter()
        .map(|id| {
            Uuid::parse_str(id.trim())
                .ok()
                .filter(|id| !id.is_nil())
                .ok_or_else(|| invalid_error("entity.ids", "entity.ids must be person UUIDs"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Sorted and deduplicated so one set of people is one selection: the
    // cursor fingerprint is taken over the selection, and the same roster in
    // another order would otherwise reject its own next page.
    parsed.sort_unstable();
    parsed.dedup();
    Ok(parsed)
}

/// The entity in its canonical form: person ids parsed and re-rendered, a
/// roster sorted and deduplicated, an unsupported shape refused.
fn normalize_entity(
    entity: MetricDrilldownEntity,
) -> Result<MetricDrilldownEntity, CanonicalError> {
    match entity {
        MetricDrilldownEntity::Person { id } => {
            let (_, person_id) = parse_person_entity(&MetricDrilldownEntity::Person { id })?;
            Ok(MetricDrilldownEntity::Person {
                id: person_id.to_string(),
            })
        }
        MetricDrilldownEntity::Persons { .. } => Ok(MetricDrilldownEntity::Persons {
            ids: parse_person_ids(&entity)?
                .into_iter()
                .map(|id| id.to_string())
                .collect(),
        }),
        MetricDrilldownEntity::Tenant {} => Ok(MetricDrilldownEntity::Tenant {}),
        MetricDrilldownEntity::Unknown => invalid("entity.type", "unsupported entity type"),
    }
}

async fn validate_common(
    db: &DatabaseConnection,
    ch: &insight_clickhouse::Client,
    tenant_id: Uuid,
    request: CommonRequest,
) -> Result<ValidatedMetricDrilldown, CanonicalError> {
    let CommonRequest {
        metric_key,
        entity,
        period,
        filters,
        display_dimensions,
        sort,
        search,
        limit,
        max_limit,
        cursor,
    } = request;
    let metric_key = normalize_metric_key("metric_key", &metric_key)?;
    let entity = normalize_entity(entity)?;
    let entity_type = entity.entity_type();
    if limit == 0 || limit > max_limit {
        return invalid("limit", format!("limit must be between 1 and {max_limit}"));
    }
    let from = parse_date("period.from", &period.from)?;
    let to = parse_date("period.to", &period.to)?;
    if from > to || (to - from).num_days() >= MAX_PERIOD_DAYS {
        return invalid(
            "period",
            format!("period must be ordered and shorter than {MAX_PERIOD_DAYS} days"),
        );
    }
    let definitions =
        load_definitions_with_ids(db, tenant_id, std::slice::from_ref(&metric_key)).await?;
    let (definition_id, definition) = definitions.get(&metric_key).cloned().ok_or_else(|| {
        MetricError::not_found("metric definition not found")
            .with_resource(&metric_key)
            .create()
    })?;
    if definition.base.entity_type != entity_type {
        return invalid(
            "entity.type",
            "entity type does not match metric definition",
        );
    }
    let filters = normalize_filters(&definition, filters)?;
    let display_dimensions = normalize_display_dimensions(&definition, display_dimensions)?;
    let plan = load_evidence_plan(db, definition_id, definition).await?;
    let snapshot_id = evidence_snapshot_id(ch, &plan.relation).await?;
    // One column set answers three questions — what may be sorted, what the
    // cursor is bound to, and what the compiler reads — so they cannot drift
    // apart between here and the query.
    let columns = presentation_columns(&plan, &filters, &display_dimensions, &entity);
    let ratio = matches!(plan.definition.spec, ComputationSpec::Ratio { .. });
    let sort = normalize_sort(&columns, sort)?;
    let search = normalize_search(search)?;
    let selection = MetricDrilldownSelection {
        metric_key,
        entity,
        period: MetricDrilldownPeriod {
            from: from.to_string(),
            to: to.to_string(),
        },
        filters,
        display_dimensions,
        sort,
        search,
    };
    let fingerprint = selection_fingerprint(tenant_id, &selection, &columns)?;
    let cursor = resume_from(
        ch,
        cursor,
        &plan,
        &fingerprint,
        &columns,
        &selection.sort,
        ratio,
    )
    .await?;
    Ok(ValidatedMetricDrilldown {
        selection,
        tenant_id,
        // The handler overwrites this from config; false is the runtime-wide
        // default (#1967) so tests compile queries in the degraded form.
        enforce_tenant_scope: false,
        from,
        to,
        limit,
        cursor,
        // Filled by the handler, which is where the names are resolved.
        search_person_ids: Vec::new(),
        plan,
        snapshot_id,
        fingerprint,
    })
}

/// The page a cursor addresses, or nothing where the caller asked for the
/// first one.
async fn resume_from(
    ch: &insight_clickhouse::Client,
    cursor: Option<String>,
    plan: &EvidencePlan,
    fingerprint: &str,
    columns: &[MetricDrilldownColumn],
    sort: &MetricDrilldownSort,
    ratio: bool,
) -> Result<Option<super::cursor::CursorKey>, CanonicalError> {
    let Some(value) = cursor else {
        return Ok(None);
    };
    let envelope = decode_cursor(&value)?;
    verify_evidence_snapshot(ch, &plan.relation, &envelope.snapshot_id).await?;
    if envelope.fingerprint != fingerprint {
        return invalid("cursor", "cursor does not match the metric selection");
    }

    // A cursor is bytes the caller holds. Its key is replayed through the
    // sorted column's own cast, and ClickHouse answers a value that will not
    // cast by refusing the QUERY — so it is refused here, as the malformed
    // cursor it is.
    let fits = column_sql(&sort.key, sort_column_type(columns, &sort.key), ratio)
        .is_some_and(|sql| sql.accepts_cursor_key(&envelope.key.sort_value));
    if !fits {
        return invalid("cursor", "cursor is malformed");
    }

    Ok(Some(envelope.key))
}

fn sort_column_type(columns: &[MetricDrilldownColumn], key: &str) -> MetricDrilldownColumnType {
    columns
        .iter()
        .find(|column| column.key == key)
        .map_or(MetricDrilldownColumnType::String, |column| column.r#type)
}

/// A sort the query cannot compile is refused rather than silently ignored: a
/// client that believes it asked for one order and is served another has no way
/// to tell.
fn normalize_sort(
    columns: &[MetricDrilldownColumn],
    sort: Option<MetricDrilldownSort>,
) -> Result<MetricDrilldownSort, CanonicalError> {
    let Some(sort) = sort else {
        return Ok(MetricDrilldownSort::newest_first());
    };
    let key = normalize_key("sort.key", &sort.key)?;
    let sortable = columns
        .iter()
        .any(|column| column.key == key && column.sortable);
    if !sortable {
        return invalid("sort.key", format!("column {key} cannot be sorted"));
    }
    Ok(MetricDrilldownSort {
        key,
        direction: sort.direction,
    })
}

fn normalize_search(search: Option<String>) -> Result<Option<String>, CanonicalError> {
    let Some(search) = search else {
        return Ok(None);
    };
    let search = search.trim();
    if search.is_empty() {
        return Ok(None);
    }
    if search.len() > MAX_SEARCH_BYTES {
        return invalid(
            "search",
            format!("search must be at most {MAX_SEARCH_BYTES} bytes"),
        );
    }
    Ok(Some(search.to_owned()))
}

async fn load_evidence_plan(
    db: &DatabaseConnection,
    definition_id: Uuid,
    definition: MetricDefinition,
) -> Result<EvidencePlan, CanonicalError> {
    let rows = EvidenceInputRow::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT i.input_role, m.measure_key, m.alias_collapse, m.evidence_granularity, \
         m.evidence_presentation, \
                s.source_key, s.evidence_ref, s.evidence_schema_status \
         FROM metric_definition_inputs i \
         INNER JOIN metric_source_measures m ON m.id = i.source_measure_id \
         INNER JOIN metric_sources s ON s.id = m.source_id \
         WHERE i.metric_definition_id = ? AND m.is_enabled = TRUE AND s.is_enabled = TRUE \
         ORDER BY i.input_role, m.measure_key",
        [Value::Bytes(Some(definition_id.as_bytes().to_vec()))],
    ))
    .all(db)
    .await
    .map_err(|error| db_error(&error))?;
    let Some((relation, source_key)) = healthy_evidence(rows.iter().map(EvidenceInputRow::health))
    else {
        return Err(evidence_unavailable());
    };
    let source_key = source_key.to_owned();
    let inputs = rows
        .into_iter()
        .map(|row| {
            let role = MetricInputRole::from_db(&row.input_role).ok_or_else(config_error)?;
            let granularity = row
                .evidence_granularity
                .as_deref()
                .and_then(EvidenceGranularity::from_db)
                .ok_or_else(config_error)?;
            let alias_collapse =
                AliasCollapse::from_db(&row.alias_collapse).ok_or_else(config_error)?;
            let presentation = StoredPresentation::read(row.evidence_presentation.as_deref())
                .or_undeclared(granularity)
                .ok_or_else(config_error)?;
            Ok(EvidenceInput {
                role,
                alias_collapse,
                presentation,
                measure_key: row.measure_key,
            })
        })
        .collect::<Result<Vec<_>, CanonicalError>>()?;
    Ok(EvidencePlan {
        definition,
        relation,
        source_key,
        inputs,
    })
}

fn normalize_filters(
    definition: &MetricDefinition,
    filters: Vec<MetricDrilldownFilter>,
) -> Result<Vec<MetricDrilldownFilter>, CanonicalError> {
    if filters.len() > MAX_FILTERS {
        return invalid("filters", format!("at most {MAX_FILTERS} filters"));
    }
    let mut normalized = Vec::with_capacity(filters.len());
    for filter in filters {
        let dimension = normalize_key("filters.dimension", &filter.dimension)?;
        if definition.allowed_dimension(&dimension).is_none() {
            return invalid(
                "filters.dimension",
                format!("dimension {dimension} is not declared by the metric"),
            );
        }
        if filter.values.is_empty() || filter.values.len() > MAX_FILTER_VALUES {
            return invalid(
                "filters.values",
                format!("between 1 and {MAX_FILTER_VALUES} values are required"),
            );
        }
        let mut values = filter
            .values
            .into_iter()
            .map(|value| value.trim().to_owned())
            .collect::<Vec<_>>();
        if values
            .iter()
            .any(|value| value.is_empty() || value.len() > MAX_FILTER_VALUE_BYTES)
        {
            return invalid("filters.values", "filter value is empty or too long");
        }
        values.sort();
        values.dedup();
        normalized.push(MetricDrilldownFilter { dimension, values });
    }
    normalized.sort_by(|left, right| left.dimension.cmp(&right.dimension));
    if normalized
        .windows(2)
        .any(|pair| pair[0].dimension == pair[1].dimension)
    {
        return invalid("filters", "duplicate dimension filter");
    }
    Ok(normalized)
}

fn normalize_display_dimensions(
    definition: &MetricDefinition,
    dimensions: Vec<String>,
) -> Result<Vec<String>, CanonicalError> {
    if dimensions.len() > MAX_DISPLAY_DIMENSIONS {
        return invalid(
            "display_dimensions",
            format!("at most {MAX_DISPLAY_DIMENSIONS} display dimensions"),
        );
    }
    let mut normalized = dimensions
        .into_iter()
        .map(|dimension| normalize_key("display_dimensions", &dimension))
        .collect::<Result<Vec<_>, _>>()?;
    for dimension in &normalized {
        if definition.allowed_dimension(dimension).is_none() {
            return invalid(
                "display_dimensions",
                format!("dimension {dimension} is not declared by the metric"),
            );
        }
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metric_definitions::ComputationSpec;
    use crate::domain::metric_drilldown::sort::MetricDrilldownSortDirection;
    use crate::domain::metric_drilldown::test_support::{commit_input, definition, input, plan};

    fn commit_plan() -> EvidencePlan {
        plan(
            ComputationSpec::Sum {
                value: input(MetricInputRole::Value, "commit_count"),
            },
            vec![commit_input(MetricInputRole::Value, "commit_count")],
        )
    }

    fn one_person() -> MetricDrilldownEntity {
        MetricDrilldownEntity::Person {
            id: "person".to_owned(),
        }
    }

    fn columns_for(entity: &MetricDrilldownEntity) -> Vec<MetricDrilldownColumn> {
        presentation_columns(&commit_plan(), &[], &[], entity)
    }

    #[test]
    fn a_selection_that_names_no_order_gets_the_newest_records_first() {
        let sort = normalize_sort(&columns_for(&one_person()), None)
            .unwrap_or_else(|error| panic!("the default order must resolve: {error}"));

        assert_eq!(sort, MetricDrilldownSort::newest_first());
        assert_eq!(sort.direction, MetricDrilldownSortDirection::Desc);
    }

    #[test]
    fn a_sort_the_query_cannot_compile_is_refused_rather_than_ignored() {
        let refused = normalize_sort(
            &columns_for(&MetricDrilldownEntity::Persons {
                ids: vec!["person".to_owned()],
            }),
            Some(MetricDrilldownSort {
                key: "person".to_owned(),
                direction: MetricDrilldownSortDirection::Asc,
            }),
        );
        assert!(refused.is_err(), "the who column is shown, not sorted");

        let unknown = normalize_sort(
            &columns_for(&one_person()),
            Some(MetricDrilldownSort {
                key: "nothing".to_owned(),
                direction: MetricDrilldownSortDirection::Asc,
            }),
        );
        assert!(unknown.is_err());
    }

    #[test]
    fn a_declared_column_sorts_under_its_own_key() {
        let sort = normalize_sort(
            &columns_for(&one_person()),
            Some(MetricDrilldownSort {
                key: " Repository ".to_owned(),
                direction: MetricDrilldownSortDirection::Asc,
            }),
        )
        .unwrap_or_else(|error| panic!("a declared column must sort: {error}"));

        assert_eq!(sort.key, "repository");
    }

    #[test]
    fn an_empty_search_narrows_nothing() {
        assert_eq!(
            normalize_search(Some("   ".to_owned()))
                .unwrap_or_else(|error| panic!("blank search must normalize: {error}")),
            None
        );
        assert_eq!(
            normalize_search(Some("  fix  ".to_owned()))
                .unwrap_or_else(|error| panic!("search must normalize: {error}")),
            Some("fix".to_owned())
        );
        assert!(normalize_search(Some("x".repeat(MAX_SEARCH_BYTES + 1))).is_err());
    }

    #[test]
    fn filters_and_display_dimensions_are_normalized() {
        let definition = definition(
            ComputationSpec::Sum {
                value: input(MetricInputRole::Value, "commit_count"),
            },
            &["repository", "category"],
        );
        let filters = normalize_filters(
            &definition,
            vec![MetricDrilldownFilter {
                dimension: " repository ".to_owned(),
                values: vec!["b".to_owned(), "a".to_owned(), "a".to_owned()],
            }],
        )
        .unwrap_or_else(|error| panic!("filter value must normalize: {error}"));
        assert_eq!(filters[0].values, ["a", "b"]);
        assert_eq!(
            normalize_display_dimensions(
                &definition,
                vec!["category".to_owned(), "category".to_owned()]
            )
            .unwrap_or_else(|error| panic!("display dimensions must normalize: {error}")),
            ["category"]
        );
        assert!(
            normalize_filters(
                &definition,
                vec![MetricDrilldownFilter {
                    dimension: "unknown".to_owned(),
                    values: vec!["value".to_owned()],
                }]
            )
            .is_err()
        );
        assert!(normalize_display_dimensions(&definition, vec!["unknown".to_owned()]).is_err());
    }

    #[test]
    fn roster_ids_are_canonical_sorted_and_deduplicated() {
        let first = Uuid::from_u128(0x019e_2830_0000_7000_8000_0000_0000_0001);
        let second = Uuid::from_u128(0x019e_2830_0000_7000_8000_0000_0000_0002);
        let parsed = parse_person_ids(&MetricDrilldownEntity::Persons {
            ids: vec![format!(" {second} "), first.to_string(), second.to_string()],
        })
        .unwrap_or_else(|error| panic!("roster must parse: {error}"));

        // One set of people is one selection: the cursor fingerprint is taken
        // over the selection, so the same roster in another order must not
        // reject its own next page.
        assert_eq!(parsed, [first, second]);
    }

    #[test]
    fn a_roster_must_name_real_people_and_stay_within_the_cap() {
        assert!(parse_person_ids(&MetricDrilldownEntity::Persons { ids: vec![] }).is_err());
        assert!(
            parse_person_ids(&MetricDrilldownEntity::Persons {
                ids: vec!["alice@example.com".to_owned()],
            })
            .is_err()
        );
        assert!(
            parse_person_ids(&MetricDrilldownEntity::Persons {
                ids: vec![Uuid::nil().to_string()],
            })
            .is_err()
        );
        assert!(
            parse_person_ids(&MetricDrilldownEntity::Persons {
                ids: (0..=MAX_ENTITY_PERSONS)
                    .map(|index| Uuid::from_u128(index as u128 + 1).to_string())
                    .collect(),
            })
            .is_err()
        );
    }

    #[test]
    fn a_single_person_reads_as_a_roster_of_one() {
        let person = Uuid::from_u128(0x019e_2830_0000_7000_8000_0000_0000_0003);
        let parsed = parse_person_ids(&MetricDrilldownEntity::Person {
            id: person.to_string(),
        })
        .unwrap_or_else(|error| panic!("person must parse: {error}"));

        assert_eq!(parsed, [person]);
    }
}
