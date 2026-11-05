const STAR_BIT: u64 = 1 << 63;
const FIELD_NAMES: [&str; 5] = ["minute", "hour", "day of month", "month", "day of week"];
const BOUNDS: [(u32, u32); 5] = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 6)];

#[derive(Debug, thiserror::Error)]
#[error("{problem}")]
pub(crate) struct Violation {
    pub field: String,
    pub problem: String,
}

impl Violation {
    pub(crate) fn new(field: impl Into<String>, problem: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            problem: problem.into(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Crons(Vec<String>);

impl Crons {
    pub(crate) fn parse(raw: &[String], min_interval_minutes: u16) -> Result<Self, Violation> {
        if !(1..=10).contains(&raw.len()) {
            return Err(Violation::new("crons", "expected 1 to 10 cron expressions"));
        }
        let mut normalized = Vec::with_capacity(raw.len());
        let mut expressions = Vec::with_capacity(raw.len());
        for (index, cron) in raw.iter().enumerate() {
            let field = format!("crons[{index}]");
            let (expression, bits) = expression(cron).map_err(|problem| {
                Violation::new(&field, format!("expression {index} {cron:?}: {problem}"))
            })?;
            if let Some(previous) = normalized.iter().position(|item| item == &expression) {
                return Err(Violation::new(
                    field,
                    format!(
                        "expression {index} {cron:?} duplicates crons[{previous}] {expression:?} after whitespace normalization"
                    ),
                ));
            }
            normalized.push(expression);
            expressions.push(bits);
        }
        frequency::check(&normalized, &expressions, min_interval_minutes)?;
        Ok(Self(normalized))
    }

    pub(crate) fn into_vec(self) -> Vec<String> {
        self.0
    }
}

pub(super) fn normalize(raw: &str) -> String {
    raw.split([' ', '\t'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn expression(raw: &str) -> Result<(String, [u64; 5]), String> {
    if raw.chars().count() > 100 {
        return Err("cron expression exceeds 100 characters".to_owned());
    }
    if raw.chars().any(|c| c.is_control() && c != '\t') {
        return Err("cron expression contains control characters or line breaks".to_owned());
    }
    let normalized = normalize(raw);
    if normalized.contains('@') {
        return Err("@ descriptors are not supported".to_owned());
    }
    if normalized.contains("TZ=") {
        return Err("TZ= and CRON_TZ= prefixes are not supported".to_owned());
    }
    let fields: Vec<_> = normalized.split(' ').collect();
    if fields.len() != 5 {
        return Err(
            "cron expression must have exactly five fields (no seconds or years)".to_owned(),
        );
    }
    let mut bits = [0; 5];
    for (index, field) in fields.iter().enumerate() {
        bits[index] = field_value(field, index)
            .map_err(|problem| format!("{}: {problem}", FIELD_NAMES[index]))?;
    }
    calendar(&bits)?;
    Ok((normalized, bits))
}

fn calendar(bits: &[u64; 5]) -> Result<(), String> {
    if (bits[2] | bits[4]) & STAR_BIT == 0 {
        return Ok(());
    }
    let max_day = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        .into_iter()
        .enumerate()
        .filter(|(index, _)| bits[3] & (1 << (index + 1)) != 0)
        .map(|(_, days)| days)
        .max()
        .unwrap_or(0);
    if (1..=max_day).any(|day| bits[2] & (1 << day) != 0) {
        return Ok(());
    }
    Err(format!(
        "expression never runs: no listed month has any listed day of month (listed months have at most {max_day} days, including leap years)"
    ))
}

fn field_value(raw: &str, index: usize) -> Result<u64, String> {
    if raw == "?" && matches!(index, 2 | 4) {
        return field_value("*", index);
    }
    let (min, max) = BOUNDS[index];
    let mut bits = 0;
    for item in raw.split(',') {
        if item.is_empty() {
            return Err("empty list item".to_owned());
        }
        let (base, step) = match item.split_once('/') {
            Some((base, step)) => {
                let step = decimal(step).ok_or("step must be a positive integer")?;
                if step == 0 {
                    return Err("step must be at least 1".to_owned());
                }
                (base, step)
            }
            None => (item, 1),
        };
        let (start, end) = if base == "*" {
            if step == 1 {
                bits |= STAR_BIT;
            }
            (min, max)
        } else if let Some((start, end)) = base.split_once('-') {
            (value(start, index)?, value(end, index)?)
        } else {
            let start = value(base, index)?;
            (start, if item.contains('/') { max } else { start })
        };
        if start > end {
            return Err("range start exceeds range end".to_owned());
        }
        for number in start..=end {
            if (number - start) % step == 0 {
                bits |= 1 << number;
            }
        }
    }
    Ok(bits)
}

fn decimal(raw: &str) -> Option<u32> {
    if raw.is_empty() || !raw.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok()
}

fn value(raw: &str, index: usize) -> Result<u32, String> {
    const MONTHS: [&str; 12] = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    const DAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
    let number = decimal(raw)
        .or_else(|| {
            let (names, offset): (&[&str], u32) = match index {
                3 => (&MONTHS, 1),
                4 => (&DAYS, 0),
                _ => return None,
            };
            names
                .iter()
                .position(|name| name.eq_ignore_ascii_case(raw))
                .and_then(|n| u32::try_from(n).ok())
                .map(|n| n + offset)
        })
        .ok_or_else(|| {
            format!("unsupported value {raw:?}; L, W, # and misplaced ? are not supported")
        })?;
    let (min, max) = BOUNDS[index];
    if !(min..=max).contains(&number) {
        return Err(format!("value {raw:?} is outside {min}–{max}"));
    }
    Ok(number)
}

mod frequency;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod frequency_tests;
