//! Turns a measure definition plus a query request into ClickHouse SQL.
//!
//! The compiler generates statements and nothing else: it returns the SQL text
//! and the parameters to bind against it, and never touches a connection.
//! Execution, row decoding, and view assembly are layers above.
//!
//! A compiled read emits the observation row shape the metric-result builders
//! already consume — `entity_id`, `metric_date`, `value`, and, when a
//! breakdown is asked for, `dimension_value` / `dimension_label` — so a view
//! assembles from either executor's rows.

#![allow(dead_code)] // tests are this module's only callers in the crate

pub mod error;
pub mod measure;
pub mod request;
