//! SeaORM's `Statement` takes only positional `?` placeholders, so [`bind_named`]
//! rewrites each `@name` to a `?` and emits its bound value once per occurrence,
//! in left-to-right order.

use sea_orm::Value;

pub fn bind_named(sql: &str, params: &[(&str, Value)]) -> anyhow::Result<(String, Vec<Value>)> {
    let mut out = String::with_capacity(sql.len());
    let mut values = Vec::new();
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '@' {
            out.push(c);
            continue;
        }
        let mut name = String::new();
        while let Some(&next) = chars.peek() {
            if next.is_ascii_alphanumeric() || next == '_' {
                name.push(next);
                chars.next();
            } else {
                break;
            }
        }
        if name.is_empty() {
            out.push('@');
            continue;
        }
        let (_, value) = params
            .iter()
            .find(|(k, _)| *k == name)
            .ok_or_else(|| anyhow::anyhow!("bind_named: no value bound for @{name}"))?;
        out.push('?');
        values.push(value.clone());
    }

    Ok((out, values))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_repeated_params_in_order() -> anyhow::Result<()> {
        let sql = "SELECT * FROM t WHERE a = @tenant_id AND b = @x OR c = @tenant_id";
        let (out, values) = bind_named(
            sql,
            &[("tenant_id", Value::from(1_i32)), ("x", Value::from(2_i32))],
        )?;
        assert_eq!(out, "SELECT * FROM t WHERE a = ? AND b = ? OR c = ?");
        assert_eq!(values.len(), 3);
        assert_eq!(values[0], Value::from(1_i32));
        assert_eq!(values[1], Value::from(2_i32));
        assert_eq!(values[2], Value::from(1_i32));
        Ok(())
    }

    #[test]
    fn stops_identifier_at_non_word_char() -> anyhow::Result<()> {
        let (out, values) = bind_named(
            "@a,@b)",
            &[("a", Value::from(7_i32)), ("b", Value::from(8_i32))],
        )?;
        assert_eq!(out, "?,?)");
        assert_eq!(values, vec![Value::from(7_i32), Value::from(8_i32)]);
        Ok(())
    }

    #[test]
    fn passes_through_bare_at() -> anyhow::Result<()> {
        let (out, values) = bind_named("a @ b", &[])?;
        assert_eq!(out, "a @ b");
        assert!(values.is_empty());
        Ok(())
    }

    #[test]
    fn errors_on_unbound_name() {
        let err = bind_named("WHERE x = @missing", &[]);
        assert!(err.is_err());
    }
}
