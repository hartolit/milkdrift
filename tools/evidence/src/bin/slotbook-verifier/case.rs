//! Operator-owned cases from the maintained specification. Neither a candidate nor its worker
//! supplies expected responses. The protected recipe's application configuration selects a case.
use milkdrift_evidence::EvidenceResult;
use serde_json::Value;

#[derive(Clone, Copy)]
pub(super) struct Case {
    pub(super) capacity: u64,
    pub(super) quantity: u32,
    pub(super) start: &'static str,
    pub(super) end: &'static str,
    pub(super) overlap: &'static str,
    pub(super) adjacent_end: &'static str,
    pub(super) before: &'static str,
    pub(super) cutoff: &'static str,
}
impl Case {
    pub(super) fn from_application(value: &Value) -> EvidenceResult<Self> {
        let resource = value["resource"].as_str().ok_or("resource absent")?;
        let capacity = value["capacity"].as_u64().ok_or("capacity absent")?;
        let notice = match value.get("cancellation_notice_seconds") {
            Some(v) => v.as_u64().ok_or("invalid cancellation notice")?,
            None => 0,
        };
        let result = match (resource, capacity, notice) {
            ("pottery", 2, 0) => Self {
                capacity,
                quantity: 1,
                start: "2027-04-10T10:00:00Z",
                end: "2027-04-10T11:00:00Z",
                overlap: "2027-04-10T10:30:00Z",
                adjacent_end: "2027-04-10T12:00:00Z",
                before: "2027-04-10T09:59:00Z",
                cutoff: "2027-04-10T10:00:00Z",
            },
            ("camera", 1, 0) => Self {
                capacity,
                quantity: 1,
                start: "2027-05-03T09:00:00Z",
                end: "2027-05-03T12:00:00Z",
                overlap: "2027-05-03T11:00:00Z",
                adjacent_end: "2027-05-03T13:00:00Z",
                before: "2027-05-03T08:59:00Z",
                cutoff: "2027-05-03T09:00:00Z",
            },
            ("tripod", 2, 3600) => Self {
                capacity,
                quantity: 2,
                start: "2027-05-04T09:00:00Z",
                end: "2027-05-04T12:00:00Z",
                overlap: "2027-05-04T11:00:00Z",
                adjacent_end: "2027-05-04T13:00:00Z",
                before: "2027-05-04T07:59:00Z",
                cutoff: "2027-05-04T08:00:00Z",
            },
            ("yoga", 4, 7200) => Self {
                capacity,
                quantity: 2,
                start: "2027-06-08T17:00:00Z",
                end: "2027-06-08T18:00:00Z",
                overlap: "2027-06-08T17:30:00Z",
                adjacent_end: "2027-06-08T19:00:00Z",
                before: "2027-06-08T14:59:00Z",
                cutoff: "2027-06-08T15:00:00Z",
            },
            ("ceramics", 6, 86400) => Self {
                capacity,
                quantity: 3,
                start: "2027-06-09T18:00:00Z",
                end: "2027-06-09T20:00:00Z",
                overlap: "2027-06-09T19:00:00Z",
                adjacent_end: "2027-06-09T21:00:00Z",
                before: "2027-06-08T17:59:00Z",
                cutoff: "2027-06-08T18:00:00Z",
            },
            _ => {
                return Err(
                    "application configuration is outside the declared Slotbook cases".into(),
                );
            }
        };
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_out_cases_keep_their_fixed_cutoffs_and_capacities() -> EvidenceResult {
        for (resource, capacity, notice, cutoff) in [
            ("camera", 1, 0, "2027-05-03T09:00:00Z"),
            ("tripod", 2, 3600, "2027-05-04T08:00:00Z"),
            ("yoga", 4, 7200, "2027-06-08T15:00:00Z"),
            ("ceramics", 6, 86400, "2027-06-08T18:00:00Z"),
        ] {
            let mut value = serde_json::json!({"resource":resource,"capacity":capacity,"cancellation_notice_seconds":notice});
            assert_eq!(Case::from_application(&value)?.cutoff, cutoff);
            value["capacity"] = serde_json::json!(capacity + 1);
            assert!(Case::from_application(&value).is_err());
        }
        Ok(())
    }
}
