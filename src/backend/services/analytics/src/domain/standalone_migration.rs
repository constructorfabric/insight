//! A numbered ClickHouse migration as a single node applies it.

const ENGINE_TOKEN_OPEN: &str = "{{ch_engine:";
const ENGINE_TOKEN_CLOSE: &str = "}}";

/// Every `{{ch_engine:<family>}}` rendered as the plain family, as
/// `ch_render_engines` renders it where the warehouse does not replicate.
pub(crate) fn standalone(sql: &str) -> String {
    let mut rendered = String::with_capacity(sql.len());
    let mut rest = sql;

    while let Some((before, after)) = rest.split_once(ENGINE_TOKEN_OPEN) {
        rendered.push_str(before);
        let Some((family, tail)) = after.split_once(ENGINE_TOKEN_CLOSE) else {
            rendered.push_str(ENGINE_TOKEN_OPEN);
            rest = after;
            break;
        };
        rendered.push_str(family);
        rest = tail;
    }

    rendered.push_str(rest);
    rendered
}

#[cfg(test)]
mod tests {
    use super::standalone;

    #[test]
    fn a_single_node_takes_the_plain_family_the_token_names() {
        let cases = [
            (
                "ENGINE = {{ch_engine:MergeTree}} ORDER BY k",
                "ENGINE = MergeTree ORDER BY k",
            ),
            (
                "ENGINE = {{ch_engine:ReplacingMergeTree}}(_version)",
                "ENGINE = ReplacingMergeTree(_version)",
            ),
            ("ENGINE = MergeTree", "ENGINE = MergeTree"),
            (
                "ENGINE = {{ch_engine:MergeTree",
                "ENGINE = {{ch_engine:MergeTree",
            ),
        ];

        for (sql, expected) in cases {
            assert_eq!(standalone(sql), expected, "rendering {sql:?}");
        }
    }
}
