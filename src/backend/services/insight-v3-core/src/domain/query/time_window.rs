use chrono::{DateTime, Datelike as _, Days, Months, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum Grain {
    Hour,
    Day,
    Week,
    Month,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum Bounds {
    Unbounded,
    Finite {
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    },
}

/// The window a run reads over.
///
/// A caller who named no range gets [`Window::Unwindowed`], which is not the
/// same as an unbounded one they did name: `inf` buckets its rows and leaves
/// out the undated, and a maximum range refuses it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum Window {
    Unwindowed,
    Requested {
        bounds: Bounds,
        grain: Option<Grain>,
    },
}

impl Window {
    pub(crate) fn legacy() -> Self {
        Self::Unwindowed
    }

    pub(crate) fn unbucketed(self) -> Self {
        match self {
            Self::Unwindowed => Self::Unwindowed,
            Self::Requested { bounds, .. } => Self::Requested {
                bounds,
                grain: None,
            },
        }
    }

    pub(crate) fn grain(&self) -> Option<Grain> {
        match self {
            Self::Unwindowed => None,
            Self::Requested { grain, .. } => *grain,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestedRange {
    PreviousDay,
    RollingDays { days: u64, grain: Grain },
    PreviousMonth,
    PreviousQuarter,
    AllTime,
    Interval { from: NaiveDate, to: NaiveDate },
}

impl RequestedRange {
    pub(crate) fn parse(value: &str) -> Result<Self, WindowError> {
        match value {
            "PDC" => Ok(Self::PreviousDay),
            "P7D" => Ok(Self::RollingDays {
                days: 7,
                grain: Grain::Day,
            }),
            "P30D" => Ok(Self::RollingDays {
                days: 30,
                grain: Grain::Day,
            }),
            "PMC" => Ok(Self::PreviousMonth),
            "PQC" => Ok(Self::PreviousQuarter),
            "P1Y" => Ok(Self::RollingDays {
                days: 365,
                grain: Grain::Month,
            }),
            "inf" => Ok(Self::AllTime),
            _ => Self::parse_interval(value),
        }
    }

    fn parse_interval(value: &str) -> Result<Self, WindowError> {
        let Some((from, to)) = value.split_once('/') else {
            return Err(WindowError::Range(value.to_owned()));
        };
        if to.contains('/') {
            return Err(WindowError::Range(value.to_owned()));
        }

        let from = canonical_date(from).ok_or_else(|| WindowError::Range(value.to_owned()))?;
        let to = canonical_date(to).ok_or_else(|| WindowError::Range(value.to_owned()))?;
        if from >= to {
            return Err(WindowError::Range(value.to_owned()));
        }

        Ok(Self::Interval { from, to })
    }

    pub(crate) fn resolve(&self, now: DateTime<Utc>) -> Result<Window, WindowError> {
        Ok(Window::Requested {
            bounds: self.bounds(now)?,
            grain: Some(self.grain()),
        })
    }

    fn bounds(&self, now: DateTime<Utc>) -> Result<Bounds, WindowError> {
        if *self == Self::AllTime {
            return Ok(Bounds::Unbounded);
        }

        if let Self::Interval { from, to } = self {
            return Ok(Bounds::Finite {
                from: midnight(*from)?,
                to: midnight(*to)?,
            });
        }

        let local_now = now.naive_utc();
        let (from, to) = match *self {
            Self::PreviousDay => {
                let to = local_now.date();
                let from = to
                    .checked_sub_days(Days::new(1))
                    .ok_or(WindowError::Overflow)?;
                (midnight(from)?, midnight(to)?)
            }
            Self::RollingDays { days, .. } => {
                let from_naive = local_now
                    .checked_sub_days(Days::new(days))
                    .ok_or(WindowError::Overflow)?;
                (from_naive.and_utc(), now)
            }
            Self::PreviousMonth => {
                let this_month = NaiveDate::from_ymd_opt(local_now.year(), local_now.month(), 1)
                    .ok_or(WindowError::Overflow)?;
                let previous = this_month
                    .checked_sub_months(Months::new(1))
                    .ok_or(WindowError::Overflow)?;
                (midnight(previous)?, midnight(this_month)?)
            }
            Self::PreviousQuarter => {
                let quarter_month = ((local_now.month() - 1) / 3) * 3 + 1;
                let this_quarter = NaiveDate::from_ymd_opt(local_now.year(), quarter_month, 1)
                    .ok_or(WindowError::Overflow)?;
                let previous = this_quarter
                    .checked_sub_months(Months::new(3))
                    .ok_or(WindowError::Overflow)?;
                (midnight(previous)?, midnight(this_quarter)?)
            }
            // Both answered above, before the clock was needed.
            Self::AllTime | Self::Interval { .. } => return Ok(Bounds::Unbounded),
        };

        Ok(Bounds::Finite { from, to })
    }

    fn grain(self) -> Grain {
        match self {
            Self::PreviousDay => Grain::Hour,
            Self::RollingDays { grain, .. } => grain,
            Self::PreviousMonth => Grain::Day,
            Self::PreviousQuarter => Grain::Week,
            Self::AllTime => Grain::Month,
            Self::Interval { from, to } => grain_for_days((to - from).num_days()),
        }
    }
}

fn canonical_date(value: &str) -> Option<NaiveDate> {
    if value.len() != 10 {
        return None;
    }
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    (parsed.format("%Y-%m-%d").to_string() == value).then_some(parsed)
}

fn grain_for_days(days: i64) -> Grain {
    match days {
        1 => Grain::Hour,
        2..=31 => Grain::Day,
        32..=92 => Grain::Week,
        _ => Grain::Month,
    }
}

fn midnight(date: NaiveDate) -> Result<DateTime<Utc>, WindowError> {
    Ok(date
        .and_hms_opt(0, 0, 0)
        .ok_or(WindowError::Overflow)?
        .and_utc())
}

/// What a caller asked a run for, before any data is read. Naming no
/// range asks for the legacy window: unbounded, unbucketed and UTC.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WindowRequest {
    range: Option<RequestedRange>,
    bucketed: bool,
}

impl WindowRequest {
    pub(crate) fn parse(range: Option<&str>, bucketed: Option<bool>) -> Result<Self, WindowError> {
        Ok(Self {
            range: range.map(RequestedRange::parse).transpose()?,
            bucketed: bucketed.unwrap_or(true),
        })
    }

    pub(crate) fn is_ranged(&self) -> bool {
        self.range.is_some()
    }

    pub(crate) fn resolve(&self, now: DateTime<Utc>) -> Result<Window, WindowError> {
        let Some(range) = self.range else {
            return Ok(Window::legacy());
        };

        let window = range.resolve(now)?;

        Ok(if self.bucketed {
            window
        } else {
            window.unbucketed()
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MaximumRange {
    Days(u32),
    Months(u32),
    Years(u32),
}

impl MaximumRange {
    pub(crate) fn parse(value: &str) -> Result<Self, WindowError> {
        let Some(body) = value.strip_prefix('P') else {
            return Err(WindowError::Maximum(value.to_owned()));
        };
        let Some((digits, unit)) = body.split_at_checked(body.len().saturating_sub(1)) else {
            return Err(WindowError::Maximum(value.to_owned()));
        };
        let amount = digits
            .parse::<u32>()
            .ok()
            .filter(|amount| *amount > 0)
            .ok_or_else(|| WindowError::Maximum(value.to_owned()))?;

        match unit {
            "D" => Ok(Self::Days(amount)),
            "M" => Ok(Self::Months(amount)),
            "Y" => Ok(Self::Years(amount)),
            _ => Err(WindowError::Maximum(value.to_owned())),
        }
    }

    /// Whether this cap admits the window. A run that named no range is
    /// not capped.
    pub(crate) fn allows(self, window: &Window) -> bool {
        let Window::Requested { bounds, .. } = window else {
            return true;
        };
        let (from, to) = match *bounds {
            Bounds::Unbounded => return false,
            Bounds::Finite { from, to } => (from, to),
        };

        let from = from.date_naive();
        let to = to.date_naive();
        let earliest = match self {
            Self::Days(days) => to.checked_sub_days(Days::new(u64::from(days))),
            Self::Months(months) => to.checked_sub_months(Months::new(months)),
            Self::Years(years) => years
                .checked_mul(12)
                .and_then(|months| to.checked_sub_months(Months::new(months))),
        };

        earliest.is_some_and(|earliest| from >= earliest)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub(crate) enum WindowError {
    #[error("unsupported or malformed time range `{0}`")]
    Range(String),
    #[error("maximum range `{0}` must be a positive day, month or year duration")]
    Maximum(String),
    #[error("time range arithmetic overflowed")]
    Overflow,
}

#[cfg(test)]
mod tests;
