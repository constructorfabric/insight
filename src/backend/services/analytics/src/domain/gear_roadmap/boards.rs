use clickhouse::Row;
use serde::{Deserialize, Serialize};

const LIST_BOARDS_SQL: &str = "SELECT
        toInt64(project_number) AS number,
        toUInt64(uniqExact(content_number)) AS cards
    FROM bronze_github.project_items
    WHERE project_number > 0
      AND content_number > 0
    GROUP BY project_number
    ORDER BY project_number";

#[derive(Debug, Clone, Row, Deserialize)]
pub(crate) struct BoardRow {
    pub(crate) number: i64,
    pub(crate) cards: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct BoardDto {
    pub(crate) number: i64,
    pub(crate) cards: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct BoardsResponse {
    pub(crate) boards: Vec<BoardDto>,
}

impl toolkit::api::api_dto::ResponseApiDto for BoardsResponse {}

pub(crate) async fn read_boards(
    ch: &insight_clickhouse::Client,
) -> Result<Vec<BoardRow>, clickhouse::error::Error> {
    ch.query(LIST_BOARDS_SQL).fetch_all::<BoardRow>().await
}

pub(crate) fn build(rows: Vec<BoardRow>) -> BoardsResponse {
    BoardsResponse {
        boards: rows
            .into_iter()
            .map(|row| BoardDto {
                number: row.number,
                cards: row.cards,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{BoardRow, LIST_BOARDS_SQL, build};

    #[test]
    fn the_statement_groups_the_cards_by_their_board() {
        assert!(LIST_BOARDS_SQL.contains("GROUP BY project_number"));
        assert!(LIST_BOARDS_SQL.contains("uniqExact(content_number)"));
    }

    #[test]
    fn a_board_without_a_number_is_not_a_board() {
        assert!(LIST_BOARDS_SQL.contains("project_number > 0"));
    }

    #[test]
    fn the_statement_binds_nothing() {
        assert_eq!(LIST_BOARDS_SQL.matches('?').count(), 0);
    }

    #[test]
    fn every_board_read_reaches_the_response() {
        let response = build(vec![
            BoardRow {
                number: 48,
                cards: 130,
            },
            BoardRow {
                number: 51,
                cards: 7,
            },
        ]);

        assert_eq!(
            response
                .boards
                .iter()
                .map(|board| (board.number, board.cards))
                .collect::<Vec<_>>(),
            vec![(48, 130), (51, 7)]
        );
    }
}
