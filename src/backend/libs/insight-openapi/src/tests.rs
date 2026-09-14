use std::error::Error;
use std::fs;

use tempfile::TempDir;
use utoipa::openapi::{InfoBuilder, OpenApiBuilder};

use super::{Difference, DriftError, canonical_json, check_committed, first_difference};

type R = Result<(), Box<dyn Error>>;

fn document(version: &str) -> utoipa::openapi::OpenApi {
    OpenApiBuilder::new()
        .info(InfoBuilder::new().title("service").version(version).build())
        .build()
}

#[derive(Debug)]
struct Layout {
    root: TempDir,
}

impl Layout {
    fn new() -> Result<Self, Box<dyn Error>> {
        let root = tempfile::tempdir()?;
        fs::create_dir_all(root.path().join("src/backend/services/service"))?;
        fs::create_dir_all(root.path().join("docs/components/backend/service"))?;
        Ok(Self { root })
    }

    fn manifest_dir(&self) -> String {
        self.root
            .path()
            .join("src/backend/services/service")
            .to_string_lossy()
            .into_owned()
    }

    fn commit(&self, text: &str) -> Result<(), Box<dyn Error>> {
        fs::write(
            self.root
                .path()
                .join("docs/components/backend/service/openapi.json"),
            text,
        )?;
        Ok(())
    }
}

#[test]
fn canonical_form_sorts_keys_and_ends_with_one_newline() -> R {
    let text = canonical_json(&document("1.0.0"))?;

    let info_at = text.find("\"info\"").ok_or("info missing")?;
    let openapi_at = text.find("\"openapi\"").ok_or("openapi missing")?;
    let paths_at = text.find("\"paths\"").ok_or("paths missing")?;
    assert!(
        info_at < openapi_at && openapi_at < paths_at,
        "keys are not sorted:\n{text}"
    );
    assert!(
        text.ends_with("}\n") && !text.ends_with("\n\n"),
        "should end with exactly one newline"
    );
    Ok(())
}

#[test]
fn canonical_form_is_the_sorted_pretty_serde_output() -> R {
    let doc = document("1.0.0");
    let expected = format!(
        "{}\n",
        serde_json::to_string_pretty(&serde_json::to_value(&doc)?)?
    );

    assert_eq!(canonical_json(&doc)?, expected);
    Ok(())
}

#[test]
fn identical_texts_have_no_difference() {
    assert_eq!(first_difference("a\nb\n", "a\nb\n"), None);
}

#[test]
fn first_differing_line_is_reported_one_based() {
    let cases = [
        ("a\nb\nc\n", "a\nx\nc\n", 2, "b", "x"),
        ("a\n", "a\nb\n", 2, "<end of file>", "b"),
        ("a\nb\n", "a\n", 2, "b", "<end of file>"),
        ("", "a\n", 1, "<end of file>", "a"),
    ];
    for (committed, generated, line, c, g) in cases {
        let expected = Difference {
            line,
            committed: c.to_owned(),
            generated: g.to_owned(),
        };
        assert_eq!(
            first_difference(committed, generated),
            Some(expected),
            "should report line {line}: {committed:?} vs {generated:?}"
        );
    }
}

#[test]
fn a_current_committed_document_passes() -> R {
    let layout = Layout::new()?;
    let doc = document("1.0.0");
    layout.commit(&canonical_json(&doc)?)?;

    check_committed(&doc, &layout.manifest_dir(), "service")?;
    Ok(())
}

#[test]
fn a_stale_committed_document_names_the_line_and_the_regenerate_command() -> R {
    let layout = Layout::new()?;
    layout.commit(&canonical_json(&document("1.0.0"))?)?;

    let err = check_committed(&document("2.0.0"), &layout.manifest_dir(), "service")
        .err()
        .ok_or("should fail on drift")?;

    let DriftError::Stale {
        line, regenerate, ..
    } = &err
    else {
        return Err(format!("should be Stale, got {err}").into());
    };
    assert!(
        *line > 1,
        "the version line is inside the document, got {line}"
    );
    assert_eq!(
        regenerate,
        "(cd src/backend && cargo run -p service -- openapi) > docs/components/backend/service/openapi.json"
    );
    assert!(
        err.to_string().contains("openapi.json is stale at line"),
        "{err}"
    );
    Ok(())
}

#[test]
fn a_committed_document_differing_only_in_line_terminators_is_stale() -> R {
    let canonical = canonical_json(&document("1.0.0"))?;
    let cases = [
        ("no trailing newline", canonical.trim_end().to_owned()),
        ("CRLF", canonical.replace('\n', "\r\n")),
    ];
    for (case, committed) in cases {
        let layout = Layout::new()?;
        layout.commit(&committed)?;

        let err = check_committed(&document("1.0.0"), &layout.manifest_dir(), "service")
            .err()
            .ok_or_else(|| format!("should fail on drift: {case}"))?;

        assert!(
            matches!(err, DriftError::StaleTerminators { .. }),
            "{case}: {err}"
        );
    }
    Ok(())
}

#[test]
fn a_missing_committed_document_is_a_read_error() -> R {
    let layout = Layout::new()?;

    let err = check_committed(&document("1.0.0"), &layout.manifest_dir(), "service")
        .err()
        .ok_or("should fail when the file is absent")?;

    assert!(matches!(err, DriftError::Read { .. }), "{err}");
    Ok(())
}
