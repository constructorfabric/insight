use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::super::compiler::{PeerQueryRow, PeriodQueryRow};

/// One entity's period value. The period builder emits a row for every
/// requested entity, so this is always a value — unknown included.
#[derive(Debug, Serialize, Deserialize)]
pub struct PeriodFragment(Option<f64>);

/// One entity's peer statistics, or `None` for "this entity has no peer pool".
/// The builder renders that state by omitting the entity, so it has to survive
/// a round trip distinctly from a cache miss.
#[derive(Debug, Serialize, Deserialize)]
pub struct PeerFragment(Option<PeerStats>);

#[derive(Debug, Serialize, Deserialize)]
struct PeerStats {
    target_value: Option<f64>,
    p25: Option<f64>,
    median: Option<f64>,
    p75: Option<f64>,
    min: Option<f64>,
    max: Option<f64>,
    n: Option<u64>,
}

impl PeriodFragment {
    pub fn from_row(row: Option<&PeriodQueryRow>) -> Self {
        Self(row.and_then(|row| row.value))
    }

    pub fn into_row(self, entity_id: String) -> PeriodQueryRow {
        PeriodQueryRow {
            entity_id,
            value: self.0,
        }
    }
}

impl PeerFragment {
    pub fn from_row(row: Option<&PeerQueryRow>) -> Self {
        Self(row.map(|row| PeerStats {
            target_value: row.target_value,
            p25: row.p25,
            median: row.median,
            p75: row.p75,
            min: row.min,
            max: row.max,
            n: row.n,
        }))
    }

    pub fn into_row(self, entity_id: String) -> Option<PeerQueryRow> {
        let stats = self.0?;
        Some(PeerQueryRow {
            entity_id,
            target_value: stats.target_value,
            p25: stats.p25,
            median: stats.median,
            p75: stats.p75,
            min: stats.min,
            max: stats.max,
            n: stats.n,
        })
    }
}

pub fn encode<T: Serialize>(value: &T) -> Option<Vec<u8>> {
    serde_json::to_vec(value).ok()
}

/// `serde_json` writes a non-finite float as `null`, which a required `f64` field
/// then refuses on the way back in. Storing such an entry would burn a key that
/// is rewritten on every request and never once readable, so it is not stored.
pub fn encode_readable<T: Serialize + DeserializeOwned>(value: &T) -> Option<Vec<u8>> {
    let bytes = encode(value)?;
    serde_json::from_slice::<T>(&bytes).ok()?;
    Some(bytes)
}

/// A fragment written by an older shape decodes to `None`, which the caller
/// treats as a miss and overwrites.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Option<T> {
    match serde_json::from_slice(bytes) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::debug!(error = %error, "discarding undecodable metric-results cache entry");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_fragment_round_trips_unknown_and_known_values() {
        for value in [None, Some(0.0), Some(-1.5)] {
            let row = PeriodQueryRow {
                entity_id: "e".to_owned(),
                value,
            };

            let bytes = encode(&PeriodFragment::from_row(Some(&row)))
                .unwrap_or_else(|| panic!("fragment must encode"));
            let decoded: PeriodFragment =
                decode(&bytes).unwrap_or_else(|| panic!("fragment must decode"));

            assert_eq!(
                decoded.into_row("e".to_owned()).value,
                value,
                "should round trip: {value:?}"
            );
        }
    }

    #[test]
    fn absent_peer_pool_round_trips_as_absent_not_as_zeroed_stats() {
        let bytes =
            encode(&PeerFragment::from_row(None)).unwrap_or_else(|| panic!("fragment must encode"));
        let decoded: PeerFragment =
            decode(&bytes).unwrap_or_else(|| panic!("fragment must decode"));

        assert!(decoded.into_row("e".to_owned()).is_none());
    }

    #[test]
    fn peer_fragment_round_trips_every_statistic() {
        let row = PeerQueryRow {
            entity_id: "e".to_owned(),
            target_value: Some(1.0),
            p25: Some(2.0),
            median: None,
            p75: Some(4.0),
            min: Some(0.5),
            max: Some(9.0),
            n: Some(7),
        };

        let bytes = encode(&PeerFragment::from_row(Some(&row)))
            .unwrap_or_else(|| panic!("fragment must encode"));
        let decoded: PeerFragment =
            decode(&bytes).unwrap_or_else(|| panic!("fragment must decode"));
        let restored = decoded
            .into_row("e".to_owned())
            .unwrap_or_else(|| panic!("stats must restore"));

        assert_eq!(restored.target_value, row.target_value);
        assert_eq!(restored.p25, row.p25);
        assert_eq!(restored.median, row.median);
        assert_eq!(restored.p75, row.p75);
        assert_eq!(restored.min, row.min);
        assert_eq!(restored.max, row.max);
        assert_eq!(restored.n, row.n);
    }

    #[test]
    fn corrupt_bytes_decode_to_none() {
        assert!(decode::<PeriodFragment>(b"not json").is_none());
    }

    /// `serde_json` turns a non-finite float into `null`, which a required `f64`
    /// will not decode — such a value must not be stored at all.
    #[test]
    fn a_value_that_could_not_be_read_back_is_not_encoded() {
        use super::super::super::dto::{HistogramBinDto, HistogramValueDto, MetricResultViewDto};

        let view = |lo: f64| MetricResultViewDto::Histogram {
            values: vec![HistogramValueDto {
                entity_id: "e".to_owned(),
                bins: vec![HistogramBinDto {
                    lo,
                    hi: 1.0,
                    count: 1,
                }],
            }],
        };

        assert!(
            encode_readable(&view(0.0)).is_some(),
            "a finite bound stores normally"
        );
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(
                encode_readable(&view(bad)).is_none(),
                "should refuse to store an unreadable bound: {bad}"
            );
        }
    }
}
