//! What the person listing answers, against a live journal.
//!
//! The listing narrows before it ranks — a cheap pass over raw observations
//! picks the candidates, the window passes run only over those — and that shape
//! is only allowed if it returns exactly what the exact filter would have
//! returned from the whole tenant. These cases are that proof: currency,
//! multi-term conjunction, literal wildcards, tenant isolation and the order the
//! page comes back in.
//!
//! INVARIANT: never `#[ignore]` these — the identity CI job runs `cargo test`
//! without `--include-ignored`, so an ignored case silently stops running.

use uuid::Uuid;

use crate::domain::resolution::EXCLUDED_PERSON;

use super::person_listing::{After, list_persons, list_persons_unnarrowed};
use super::test_fixture::{Fixture, fixture_or_skip};

type TestResult = anyhow::Result<()>;

const PAGE: u64 = 100;

async fn find(f: &Fixture, query: &str) -> anyhow::Result<Vec<Uuid>> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_owned).collect();
    let rows = list_persons(&f.db, f.tenant, &terms, &[], None, None, PAGE).await?;
    Ok(rows.into_iter().map(|row| row.person_id).collect())
}

#[tokio::test]
async fn a_term_reaches_every_searched_value_of_a_person() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let person = f.person("ada@listing.test").await?;
    f.observed(person, "username", "ada-git").await?;
    f.observed(person, "display_name", "Ada Example").await?;
    f.observed(person, "first_name", "Adalovelace").await?;
    f.observed(person, "last_name", "Byronesque").await?;

    for term in [
        "ada@listing.test",
        "ada-git",
        "Ada Example",
        "Adalovelace",
        "Byronesque",
    ] {
        assert_eq!(find(&f, term).await?, vec![person], "not found by {term:?}");
    }
    Ok(())
}

#[tokio::test]
async fn a_term_matches_regardless_of_case() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let person = f.person("Mixed.Case@listing.test").await?;

    assert_eq!(find(&f, "mixed.case@LISTING.test").await?, vec![person]);
    Ok(())
}

/// The HR number is stored and served on the profile, and deliberately not
/// searched: nobody looks a colleague up by it, and every searched value type
/// costs a pass over that slice of the journal.
#[tokio::test]
async fn an_employee_id_is_not_a_searchable_value() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let person = f.person("numbered@listing.test").await?;
    f.observed(person, "employee_id", "E900123").await?;

    assert!(find(&f, "E900123").await?.is_empty());
    assert_eq!(
        find(&f, "numbered@listing.test").await?,
        vec![person],
        "the person is still reachable by a searched value"
    );
    Ok(())
}

#[tokio::test]
async fn a_superseded_value_stops_matching_its_old_owner() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let person = f.person("current@listing.test").await?;
    f.observed_at(person, "email", "former@listing.test", 86_400)
        .await?;
    f.observed_at(person, "email", "current@listing.test", 60)
        .await?;

    assert!(
        find(&f, "former@listing.test").await?.is_empty(),
        "the superseded address must not find them"
    );
    assert_eq!(find(&f, "current@listing.test").await?, vec![person]);
    Ok(())
}

#[tokio::test]
async fn two_terms_must_both_match_the_same_person() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let both = f.person("both@listing.test").await?;
    f.observed(both, "first_name", "Adamant").await?;
    f.observed(both, "last_name", "Byronesque").await?;
    let one = f.person("one@listing.test").await?;
    f.observed(one, "first_name", "Adamant").await?;
    f.observed(one, "last_name", "Otherwise").await?;

    assert_eq!(find(&f, "Adamant Byronesque").await?, vec![both]);
    Ok(())
}

/// The conjunction is per person, not per row: one term may land on the name and
/// the next on the address. A pre-filter that required one observation to carry
/// both terms would answer nobody.
#[tokio::test]
async fn terms_may_land_on_different_values_of_one_person() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let person = f.person("split@listing.test").await?;
    f.observed(person, "first_name", "Adamant").await?;

    assert_eq!(find(&f, "Adamant split@listing.test").await?, vec![person]);
    Ok(())
}

#[tokio::test]
async fn a_wildcard_in_a_term_is_matched_literally() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let literal = f.person("literal@listing.test").await?;
    f.observed(literal, "display_name", "Milestone 50% Owner")
        .await?;
    let decoy = f.person("decoy@listing.test").await?;
    f.observed(decoy, "display_name", "Milestone 50 Owner")
        .await?;

    assert_eq!(find(&f, "50%").await?, vec![literal]);
    Ok(())
}

#[tokio::test]
async fn an_id_named_search_answers_exactly_that_person() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let wanted = f.person("wanted@listing.test").await?;
    f.person("other@listing.test").await?;
    let emailless = f.emailless_person().await?;

    let named = list_persons(&f.db, f.tenant, &[], &[wanted], None, None, PAGE).await?;
    assert_eq!(
        named.into_iter().map(|r| r.person_id).collect::<Vec<_>>(),
        vec![wanted]
    );
    // The id is the only way to a person the journal holds no values for.
    let by_id = list_persons(&f.db, f.tenant, &[], &[emailless], None, None, PAGE).await?;
    assert_eq!(
        by_id.into_iter().map(|r| r.person_id).collect::<Vec<_>>(),
        vec![emailless]
    );
    Ok(())
}

#[tokio::test]
async fn the_excluded_sentinel_is_never_listed_as_a_person() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    f.person("real@person-listing.test").await?;
    f.person_as(EXCLUDED_PERSON, "excluded@person-listing.test")
        .await?;

    let browsed = list_persons(&f.db, f.tenant, &[], &[], None, None, 1_000).await?;

    assert_eq!(browsed.len(), 1, "the sentinel is a bucket, not a person");
    assert!(
        browsed.iter().all(|row| row.person_id != EXCLUDED_PERSON),
        "the excluded bucket must never be offered as somebody to pick"
    );
    Ok(())
}

#[tokio::test]
async fn another_tenants_person_is_never_listed() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let ours = f.person("shared-name@listing.test").await?;
    let elsewhere = f.in_another_tenant();
    let theirs = elsewhere.person("shared-name@listing.test").await?;

    assert_eq!(find(&f, "shared-name@listing.test").await?, vec![ours]);
    assert_eq!(
        find(&elsewhere, "shared-name@listing.test").await?,
        vec![theirs]
    );
    Ok(())
}

#[tokio::test]
async fn the_page_is_ordered_by_the_label_the_row_shows() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // Labels chosen so the order cannot come from the ids or the insert order:
    // display name wins, else the composed name, else the address.
    let zoe = f.person("aaa-first-by-address@listing.test").await?;
    f.observed(zoe, "display_name", "Zoe Displayed").await?;
    let mid = f.person("mmm-by-address@listing.test").await?;
    let byname = f.person("zzz-last-by-address@listing.test").await?;
    f.observed(byname, "first_name", "Bertha").await?;
    f.observed(byname, "last_name", "Composed").await?;
    let nameless = f.emailless_person().await?;

    let rows = list_persons(&f.db, f.tenant, &[], &[], None, None, PAGE).await?;
    let order: Vec<Uuid> = rows.into_iter().map(|r| r.person_id).collect();

    assert_eq!(
        order,
        vec![byname, mid, zoe, nameless],
        "expected Bertha Composed < mmm-by-address < Zoe Displayed < the unnamed"
    );
    Ok(())
}

#[tokio::test]
async fn paging_one_row_at_a_time_retraces_the_same_order() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    for name in ["Anna Paged", "Boris Paged", "Clara Paged"] {
        let person = f
            .person(&format!(
                "{}@listing.test",
                name.to_lowercase().replace(' ', "-")
            ))
            .await?;
        f.observed(person, "display_name", name).await?;
    }

    let whole: Vec<(Uuid, String)> = list_persons(&f.db, f.tenant, &[], &[], None, None, PAGE)
        .await?
        .into_iter()
        .map(|r| (r.person_id, r.order_key))
        .collect();

    let mut walked: Vec<Uuid> = Vec::new();
    let mut resume: Option<(String, Uuid)> = None;
    for _ in 0..whole.len() {
        let after = resume.as_ref().map(|(key, id)| After {
            order_key: key,
            person_id: *id,
        });
        let page = list_persons(&f.db, f.tenant, &[], &[], None, after, 1).await?;
        let Some(row) = page.into_iter().next() else {
            break;
        };
        walked.push(row.person_id);
        resume = Some((row.order_key, row.person_id));
    }

    assert_eq!(
        walked,
        whole.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        "a one-row walk must retrace the whole page in the same order"
    );
    Ok(())
}

/// The fallback a too-common term lands in must answer what the narrowed path
/// answers. Only the probe's own cap decides which runs, so the two shapes are
/// compared here directly rather than through a roster big enough to trip it.
#[tokio::test]
async fn the_unnarrowed_fallback_answers_exactly_what_the_probe_narrowed_to() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let hit = f.person("fallback-hit@listing.test").await?;
    f.observed(hit, "display_name", "Fallback Comparable")
        .await?;
    f.observed_at(hit, "email", "gone@listing.test", 86_400)
        .await?;
    f.observed_at(hit, "email", "fallback-hit@listing.test", 60)
        .await?;
    let miss = f.person("unrelated@listing.test").await?;
    f.observed(miss, "display_name", "Unrelated Person").await?;

    for query in [
        "fallback",
        "Fallback Comparable",
        "gone@listing.test",
        "50%",
        "comparable fallback-hit@listing.test",
    ] {
        let terms: Vec<String> = query.split_whitespace().map(str::to_owned).collect();
        let narrowed = list_persons(&f.db, f.tenant, &terms, &[], None, None, PAGE).await?;
        let whole_tenant = list_persons_unnarrowed(&f.db, f.tenant, &terms, None, PAGE).await?;

        assert_eq!(
            narrowed.iter().map(|r| r.person_id).collect::<Vec<_>>(),
            whole_tenant.iter().map(|r| r.person_id).collect::<Vec<_>>(),
            "the two shapes disagree on {query:?}"
        );
    }
    assert_eq!(find(&f, "fallback").await?, vec![hit]);
    Ok(())
}
