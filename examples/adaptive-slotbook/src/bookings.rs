use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
};

// This finite example refuses new rows at its declared storage bound, retaining cancellation
// identities so an old successful cancellation cannot change meaning after later bookings.
const MAX_BOOKINGS: usize = 4096;
const MAX_STORE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(transparent)]
pub(super) struct Timestamp(String);
impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(|_| serde::de::Error::custom("invalid UTC timestamp"))
    }
}
impl PartialEq for Timestamp {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for Timestamp {}
impl PartialOrd for Timestamp {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Timestamp {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key().cmp(&other.key())
    }
}
impl Timestamp {
    fn seconds(&self) -> u64 {
        // Parsing has already checked the Gregorian fields. An ordinal avoids local time zones
        // and makes a cancellation notice period work across midnight, month and leap boundaries.
        let digits = |start: usize, end: usize| {
            self.0.as_bytes()[start..end]
                .iter()
                .fold(0_u64, |value, digit| value * 10 + u64::from(digit - b'0'))
        };
        let year = digits(0, 4);
        let prior = year - 1;
        let month = digits(5, 7);
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let mut days = prior * 365 + prior / 4 - prior / 100 + prior / 400;
        for m in 1..month {
            days += match m {
                2 => {
                    if leap {
                        29
                    } else {
                        28
                    }
                }
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };
        }
        (days + digits(8, 10) - 1) * 86_400
            + digits(11, 13) * 3600
            + digits(14, 16) * 60
            + digits(17, 19)
    }

    fn before_notice_boundary(&self, start: &Self, notice_seconds: u32) -> bool {
        (self.seconds() + u64::from(notice_seconds), self.key().1)
            < (start.seconds(), start.key().1)
    }

    // Every constructor validates ASCII calendar fields. Trim fractional zeroes so equal
    // instants compare equally while preserving the caller's representation in HTTP and storage.
    fn key(&self) -> (&str, &str) {
        let fraction = self.0[19..].strip_prefix('.').unwrap_or("Z");
        (
            &self.0[..19],
            fraction.trim_end_matches('Z').trim_end_matches('0'),
        )
    }
    pub(super) fn parse(value: &str) -> Result<Self, Failure> {
        let b = value.as_bytes();
        if !(20..=32).contains(&b.len())
            || b[4] != b'-'
            || b[7] != b'-'
            || b[10] != b'T'
            || b[13] != b':'
            || b[16] != b':'
            || b.last() != Some(&b'Z')
            || (b.len() != 20
                && (b[19] != b'.'
                    || b.len() < 22
                    || !b[20..b.len() - 1].iter().all(u8::is_ascii_digit)))
            || b[..19]
                .iter()
                .enumerate()
                .any(|(i, c)| ![4, 7, 10, 13, 16].contains(&i) && !c.is_ascii_digit())
        {
            return Err(Failure::Invalid);
        }
        let number = |range: std::ops::Range<usize>| {
            value[range].parse::<u32>().map_err(|_| Failure::Invalid)
        };
        let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let days = match month {
            2 => {
                if leap {
                    29
                } else {
                    28
                }
            }
            4 | 6 | 9 | 11 => 30,
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            _ => 0,
        };
        if year == 0
            || day == 0
            || day > days
            || number(11..13)? > 23
            || number(14..16)? > 59
            || number(17..19)? > 59
        {
            return Err(Failure::Invalid);
        }
        Ok(Self(value.to_owned()))
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reservation {
    pub(super) name: String,
    pub(super) start: String,
    pub(super) end: String,
    pub(super) quantity: u32,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Booking {
    pub(super) id: u64,
    pub(super) name: String,
    pub(super) start: Timestamp,
    pub(super) end: Timestamp,
    pub(super) quantity: u32,
    pub(super) cancelled: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema_version: u32,
    next_id: u64,
    bookings: Vec<Booking>,
}

#[derive(Debug)]
pub(super) enum Failure {
    Invalid,
    Conflict,
    Missing,
    Full,
    Unavailable,
}

pub(super) struct Bookings {
    capacity: u32,
    cancellation_notice_seconds: u32,
    path: Option<PathBuf>,
    snapshot: Snapshot,
    unavailable: bool,
}
impl Bookings {
    pub(super) fn open(
        capacity: u32,
        cancellation_notice_seconds: u32,
        path: Option<PathBuf>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if capacity == 0 {
            return Err("capacity must be positive".into());
        }
        let snapshot = if let Some(path) = &path
            && path.exists()
        {
            let mut bytes = Vec::new();
            fs::File::open(path)?
                .take(MAX_STORE_BYTES + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_STORE_BYTES {
                return Err("booking store exceeds its bound".into());
            }
            let s: Snapshot = serde_json::from_slice(&bytes)?;
            let mut ids = std::collections::BTreeSet::new();
            if s.schema_version != 1
                || s.next_id == 0
                || s.bookings.len() > MAX_BOOKINGS
                || s.bookings.iter().any(|b| {
                    b.id == 0
                        || b.id >= s.next_id
                        || !ids.insert(b.id)
                        || b.name.is_empty()
                        || b.name.len() > 128
                        || b.quantity == 0
                        || b.quantity > capacity
                        || Timestamp::parse(&b.start.0).is_err()
                        || Timestamp::parse(&b.end.0).is_err()
                        || b.start >= b.end
                })
            {
                return Err("invalid retained booking state".into());
            }
            s
        } else {
            Snapshot {
                schema_version: 1,
                next_id: 1,
                bookings: Vec::new(),
            }
        };
        Ok(Self {
            capacity,
            cancellation_notice_seconds,
            path,
            snapshot,
            unavailable: false,
        })
    }
    fn available(&self) -> Result<(), Failure> {
        if self.unavailable {
            Err(Failure::Unavailable)
        } else {
            Ok(())
        }
    }
    pub(super) fn active(&self) -> Result<Vec<Booking>, Failure> {
        self.available()?;
        Ok(self
            .snapshot
            .bookings
            .iter()
            .filter(|b| !b.cancelled)
            .cloned()
            .collect())
    }
    pub(super) fn remaining(&self, start: &Timestamp, end: &Timestamp) -> Result<u32, Failure> {
        self.available()?;
        if start >= end {
            return Err(Failure::Invalid);
        }
        let mut events = Vec::new();
        for row in self.snapshot.bookings.iter().filter(|b| !b.cancelled) {
            let lo = start.max(&row.start);
            let hi = end.min(&row.end);
            if lo < hi {
                events.push((lo, i64::from(row.quantity)));
                events.push((hi, -i64::from(row.quantity)));
            }
        }
        events.sort_unstable(); // Negative deltas at an endpoint preserve half-open intervals.
        let (mut current, mut peak) = (0_i64, 0_i64);
        for (_, delta) in events {
            current += delta;
            peak = peak.max(current);
        }
        u32::try_from(i64::from(self.capacity) - peak).map_err(|_| Failure::Unavailable)
    }
    pub(super) fn create(&mut self, request: Reservation) -> Result<u64, Failure> {
        let (start, end) = (
            Timestamp::parse(&request.start)?,
            Timestamp::parse(&request.end)?,
        );
        if request.name.is_empty() || request.name.len() > 128 || request.quantity == 0 {
            return Err(Failure::Invalid);
        }
        if self.remaining(&start, &end)? < request.quantity {
            return Err(Failure::Conflict);
        }
        if self.snapshot.bookings.len() >= MAX_BOOKINGS {
            return Err(Failure::Full);
        }
        let mut next = self.snapshot.clone();
        let id = next.next_id;
        next.next_id = id.checked_add(1).ok_or(Failure::Full)?;
        next.bookings.push(Booking {
            id,
            name: request.name,
            start,
            end,
            quantity: request.quantity,
            cancelled: false,
        });
        self.commit(next)?;
        Ok(id)
    }
    pub(super) fn cancel(&mut self, id: u64, clock: &Timestamp) -> Result<(), Failure> {
        self.available()?;
        let mut next = self.snapshot.clone();
        let row = next
            .bookings
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or(Failure::Missing)?;
        if row.cancelled {
            return Ok(());
        }
        if !clock.before_notice_boundary(&row.start, self.cancellation_notice_seconds) {
            return Err(Failure::Conflict);
        }
        row.cancelled = true;
        self.commit(next)
    }
    fn commit(&mut self, next: Snapshot) -> Result<(), Failure> {
        if let Some(path) = &self.path {
            let persist = || -> Result<(), Box<dyn std::error::Error>> {
                let parent = path.parent().ok_or("store parent absent")?;
                let bytes = serde_json::to_vec(&next)?;
                if bytes.len() as u64 > MAX_STORE_BYTES {
                    return Err("booking store exceeds bound".into());
                }
                let mut file = tempfile::NamedTempFile::new_in(parent)?;
                file.write_all(&bytes)?;
                file.as_file().sync_all()?;
                file.persist(path)?;
                #[cfg(unix)]
                fs::File::open(parent)?.sync_all()?;
                Ok(())
            };
            if persist().is_err() {
                // A failed sync may follow rename. Stop accepting work until reopen establishes
                // the actual persisted snapshot rather than treating an uncertain write as absent.
                self.unavailable = true;
                return Err(Failure::Unavailable);
            }
        }
        self.snapshot = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_notice_is_strict_across_calendar_and_fraction_boundaries()
    -> Result<(), Box<dyn std::error::Error>> {
        for (start, notice, before, cutoff) in [
            (
                "2027-05-04T09:00:00Z",
                3600,
                "2027-05-04T07:59:00Z",
                "2027-05-04T08:00:00Z",
            ),
            (
                "2027-06-08T17:00:00Z",
                7200,
                "2027-06-08T14:59:00Z",
                "2027-06-08T15:00:00Z",
            ),
            (
                "2028-03-01T00:00:00.1Z",
                86400,
                "2028-02-29T00:00:00.09Z",
                "2028-02-29T00:00:00.10Z",
            ),
            (
                "2027-01-01T00:00:00Z",
                86400,
                "2026-12-30T23:59:59Z",
                "2026-12-31T00:00:00Z",
            ),
        ] {
            let parse = |s| Timestamp::parse(s).map_err(|e| format!("{e:?}"));
            assert!(parse(before)?.before_notice_boundary(&parse(start)?, notice));
            assert!(!parse(cutoff)?.before_notice_boundary(&parse(start)?, notice));
            let mut store = Bookings::open(2, notice, None)?;
            let id = store
                .create(Reservation {
                    name: "Ada".into(),
                    start: start.into(),
                    end: "2029-01-01T00:00:00Z".into(),
                    quantity: 2,
                })
                .map_err(|e| format!("{e:?}"))?;
            assert!(matches!(
                store.cancel(id, &parse(cutoff)?),
                Err(Failure::Conflict)
            ));
            store
                .cancel(id, &parse(before)?)
                .map_err(|e| format!("{e:?}"))?;
            store
                .cancel(id, &parse(cutoff)?)
                .map_err(|e| format!("{e:?}"))?;
        }
        Ok(())
    }
    #[test]
    fn timestamps_refuse_invalid_calendar_values_and_noncanonical_forms() {
        for value in [
            "2027-02-29T00:00:00Z",
            "2028-04-31T00:00:00Z",
            "2028-01-01T24:00:00Z",
            "2028-01-01T00:00:00+00:00",
            "é028-01-01T00:00:00Z",
        ] {
            assert!(Timestamp::parse(value).is_err());
        }
        assert!(Timestamp::parse("2028-02-29T23:59:59Z").is_ok());
        assert!(Timestamp::parse("2028-02-29T23:59:59.Z").is_err());
        assert_eq!(
            Timestamp::parse("2028-02-29T23:59:59Z").ok(),
            Timestamp::parse("2028-02-29T23:59:59.000Z").ok()
        );
        assert!(
            Timestamp::parse("2028-02-29T23:59:59.01Z").ok()
                > Timestamp::parse("2028-02-29T23:59:59Z").ok()
        );
        assert!(
            Timestamp::parse("2028-02-29T23:59:59.1Z").ok()
                < Timestamp::parse("2028-02-29T23:59:59.11Z").ok()
        );
    }
    #[test]
    fn persisted_reservations_preserve_capacity_cancellation_and_ids()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("bookings.json");
        let request = Reservation {
            name: "Ada".into(),
            start: "2027-04-10T10:00:00Z".into(),
            end: "2027-04-10T11:00:00Z".into(),
            quantity: 1,
        };
        let mut store = Bookings::open(1, 0, Some(path.clone()))?;
        let id = store
            .create(request.clone())
            .map_err(|e| format!("{e:?}"))?;
        assert!(matches!(
            store.create(request.clone()),
            Err(Failure::Conflict)
        ));
        drop(store);
        let mut reopened = Bookings::open(1, 0, Some(path))?;
        assert!(matches!(
            reopened.create(request.clone()),
            Err(Failure::Conflict)
        ));
        let clock = Timestamp::parse("2027-04-10T09:00:00Z").map_err(|e| format!("{e:?}"))?;
        reopened.cancel(id, &clock).map_err(|e| format!("{e:?}"))?;
        reopened.cancel(id, &clock).map_err(|e| format!("{e:?}"))?;
        assert!(reopened.create(request).map_err(|e| format!("{e:?}"))? > id);
        Ok(())
    }

    #[test]
    fn failed_persistence_requires_reopen_before_accepting_more_work()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("bookings.json");
        let mut store = Bookings::open(1, 0, Some(path.clone()))?;
        // A directory at the snapshot destination makes the real atomic replacement fail.
        fs::create_dir(&path)?;
        let request = Reservation {
            name: "Ada".into(),
            start: "2027-04-10T10:00:00Z".into(),
            end: "2027-04-10T11:00:00Z".into(),
            quantity: 1,
        };
        assert!(matches!(
            store.create(request.clone()),
            Err(Failure::Unavailable)
        ));
        fs::remove_dir(&path)?;
        assert!(matches!(
            store.create(request.clone()),
            Err(Failure::Unavailable)
        ));
        assert!(matches!(store.active(), Err(Failure::Unavailable)));
        let mut reopened = Bookings::open(1, 0, Some(path))?;
        assert!(reopened.active().map_err(|e| format!("{e:?}"))?.is_empty());
        assert_eq!(reopened.create(request).map_err(|e| format!("{e:?}"))?, 1);
        Ok(())
    }
}
