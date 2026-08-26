//! Scalar-expression validator for measure `value_expr` / `subject_expr`
//! fragments. The fragment is parsed with sqlparser's ClickHouse dialect and
//! admitted only if its AST contains exclusively: bare column references,
//! literals, arithmetic, and allowlisted functions. Subqueries, table
//! references, casts, and every other construct are rejected at the parser, so
//! SQL's fully specified semantics come for free while safety comes from the
//! allowlist — and every widening of the allowlist is an explicit reviewed
//! change to [`ALLOWED_FUNCTIONS`].

use std::collections::BTreeSet;

use sqlparser::ast::{BinaryOperator, Expr, UnaryOperator};
use sqlparser::dialect::ClickHouseDialect;
use sqlparser::parser::Parser;
use sqlparser::tokenizer::Token;

/// Functions a scalar expression may call. Deliberately empty until the first
/// extraction proves a need; each addition is a reviewed change.
const ALLOWED_FUNCTIONS: &[&str] = &[];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScalarExprError {
    #[error("expression is empty")]
    Empty,
    #[error("expression does not parse: {0}")]
    Parse(String),
    #[error("expression has trailing input after the first complete expression")]
    TrailingInput,
    #[error("function `{0}` is not on the scalar-expression allowlist")]
    FunctionNotAllowed(String),
    #[error("qualified column references are not allowed; a measure reads one dataset")]
    QualifiedColumn,
    #[error("quoted identifiers are not allowed; catalog fields are bare snake_case")]
    QuotedIdentifier,
    #[error("operator `{0}` is not arithmetic")]
    NonArithmeticOperator(String),
    #[error("subqueries are not allowed in a scalar expression")]
    Subquery,
    #[error("{0} is not allowed in a scalar expression")]
    UnsupportedConstruct(&'static str),
}

/// A validated fragment: the columns it references, for catalog binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalarExpr {
    pub columns: BTreeSet<String>,
}

pub fn validate_scalar_expr(input: &str) -> Result<ScalarExpr, ScalarExprError> {
    if input.trim().is_empty() {
        return Err(ScalarExprError::Empty);
    }
    let dialect = ClickHouseDialect {};
    let mut parser = Parser::new(&dialect)
        .try_with_sql(input)
        .map_err(|e| ScalarExprError::Parse(e.to_string()))?;
    let expr = parser
        .parse_expr()
        .map_err(|e| ScalarExprError::Parse(e.to_string()))?;
    if parser.peek_token().token != Token::EOF {
        return Err(ScalarExprError::TrailingInput);
    }
    let mut columns = BTreeSet::new();
    walk(&expr, &mut columns)?;
    Ok(ScalarExpr { columns })
}

fn walk(expr: &Expr, columns: &mut BTreeSet<String>) -> Result<(), ScalarExprError> {
    match expr {
        Expr::Identifier(ident) => {
            if ident.quote_style.is_some() {
                return Err(ScalarExprError::QuotedIdentifier);
            }
            columns.insert(ident.value.clone());
            Ok(())
        }
        Expr::CompoundIdentifier(_) => Err(ScalarExprError::QualifiedColumn),
        Expr::Value(_) => Ok(()),
        Expr::BinaryOp { left, op, right } => {
            if !is_arithmetic(op) {
                return Err(ScalarExprError::NonArithmeticOperator(op.to_string()));
            }
            walk(left, columns)?;
            walk(right, columns)
        }
        Expr::UnaryOp { op, expr } => match op {
            UnaryOperator::Plus | UnaryOperator::Minus => walk(expr, columns),
            other => Err(ScalarExprError::NonArithmeticOperator(other.to_string())),
        },
        Expr::Nested(inner) => walk(inner, columns),
        Expr::Function(function) => {
            let name = function.name.to_string();
            if ALLOWED_FUNCTIONS
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(&name))
            {
                return Err(ScalarExprError::UnsupportedConstruct(
                    "an allowlisted function (argument walking lands with the first allowlisted function)",
                ));
            }
            Err(ScalarExprError::FunctionNotAllowed(name))
        }
        Expr::Subquery(_) | Expr::Exists { .. } | Expr::InSubquery { .. } => {
            Err(ScalarExprError::Subquery)
        }
        Expr::Cast { .. } => Err(ScalarExprError::UnsupportedConstruct("a cast")),
        Expr::Case { .. } => Err(ScalarExprError::UnsupportedConstruct("a CASE expression")),
        Expr::Tuple(_) => Err(ScalarExprError::UnsupportedConstruct("a tuple")),
        Expr::InList { .. } | Expr::Between { .. } | Expr::Like { .. } | Expr::ILike { .. } => {
            Err(ScalarExprError::UnsupportedConstruct("a predicate"))
        }
        _ => Err(ScalarExprError::UnsupportedConstruct(
            "an unsupported expression construct",
        )),
    }
}

fn is_arithmetic(op: &BinaryOperator) -> bool {
    matches!(
        op,
        BinaryOperator::Plus
            | BinaryOperator::Minus
            | BinaryOperator::Multiply
            | BinaryOperator::Divide
            | BinaryOperator::Modulo
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn columns(input: &str) -> Vec<String> {
        validate_scalar_expr(input)
            .expect("validates")
            .columns
            .into_iter()
            .collect()
    }

    #[test]
    fn arithmetic_over_columns_and_literals_is_admitted() {
        assert_eq!(
            columns("lines_added + lines_removed"),
            ["lines_added", "lines_removed"]
        );
        assert_eq!(
            columns("(gross_cents - discount_cents) / 100.0"),
            ["discount_cents", "gross_cents"]
        );
        assert_eq!(columns("-net_change"), ["net_change"]);
        assert_eq!(columns("value % 7"), ["value"]);
        assert_eq!(columns("1 + 2.5"), Vec::<String>::new());
    }

    #[test]
    fn every_function_is_rejected_while_the_allowlist_is_empty() {
        for expr in [
            "count(x)",
            "now()",
            "sleep(1)",
            "toDate(x)",
            "coalesce(a, 0)",
        ] {
            assert!(
                matches!(
                    validate_scalar_expr(expr),
                    Err(ScalarExprError::FunctionNotAllowed(_))
                ),
                "{expr}"
            );
        }
    }

    #[test]
    fn table_shaped_constructs_are_rejected() {
        assert_eq!(
            validate_scalar_expr("(SELECT 1)"),
            Err(ScalarExprError::Subquery)
        );
        assert_eq!(
            validate_scalar_expr("t.column_a"),
            Err(ScalarExprError::QualifiedColumn)
        );
    }

    #[test]
    fn non_arithmetic_operators_are_rejected() {
        for expr in ["a = b", "a AND b", "a || b", "a > 1"] {
            assert!(
                matches!(
                    validate_scalar_expr(expr),
                    Err(ScalarExprError::NonArithmeticOperator(_))
                ),
                "{expr}"
            );
        }
    }

    #[test]
    fn statements_and_trailing_input_are_rejected() {
        assert_eq!(
            validate_scalar_expr("a; DROP TABLE users"),
            Err(ScalarExprError::TrailingInput)
        );
        assert_eq!(
            validate_scalar_expr("a b"),
            Err(ScalarExprError::TrailingInput)
        );
        assert!(matches!(
            validate_scalar_expr("SETTINGS max_threads = 1"),
            Err(ScalarExprError::Parse(_) | ScalarExprError::TrailingInput)
        ));
    }

    #[test]
    fn casts_case_and_quoted_identifiers_are_rejected() {
        assert!(matches!(
            validate_scalar_expr("CAST(a AS Float64)"),
            Err(ScalarExprError::UnsupportedConstruct(_))
        ));
        assert!(matches!(
            validate_scalar_expr("CASE WHEN a THEN 1 ELSE 0 END"),
            Err(ScalarExprError::UnsupportedConstruct(_))
        ));
        assert_eq!(
            validate_scalar_expr("\"weird column\""),
            Err(ScalarExprError::QuotedIdentifier)
        );
    }

    #[test]
    fn empty_input_is_rejected() {
        assert_eq!(validate_scalar_expr("  "), Err(ScalarExprError::Empty));
    }
}
