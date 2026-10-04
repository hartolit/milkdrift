use super::{
    require,
    service::{Service, podman, remove_cidfile},
};
use milkdrift_evidence::EvidenceResult;
use reqwest::Method;
use serde_json::{Value, json};

fn booking(
    s: &Service,
    name: &str,
    start: &str,
    end: &str,
    quantity: u32,
    auth: Option<&str>,
) -> EvidenceResult<(u16, Value)> {
    s.request(
        Method::POST,
        "/reservations",
        Some(json!({"name":name,"start":start,"end":end,"quantity":quantity})),
        auth,
    )
}
fn book(s: &Service) -> EvidenceResult<(u16, Value)> {
    booking(s, "Ada", s.case.start, s.case.end, 1, Some(&s.token))
}

fn booking_quantity(item: &Value) -> EvidenceResult<u32> {
    Ok(u32::try_from(
        item.get("quantity")
            .and_then(Value::as_u64)
            .ok_or("quantity absent")?,
    )?)
}

fn fill(s: &Service, mut quantity: u64) -> EvidenceResult {
    while quantity > 0 {
        let count = u32::try_from(quantity.min(u64::from(s.case.quantity)))?;
        require(
            booking(s, "Bo", s.case.start, s.case.end, count, Some(&s.token))?.0 == 201,
            "declared quantity booking failed",
        )?;
        quantity -= u64::from(count);
    }
    Ok(())
}
fn remaining(s: &Service, start: &str, end: &str) -> EvidenceResult<u64> {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("start", start)
        .append_pair("end", end)
        .finish();
    let (status, body) = s.request(Method::GET, &format!("/availability?{query}"), None, None)?;
    require(
        status == 200
            && body.as_object().is_some_and(|v| v.len() == 4)
            && body.pointer("/resource").ok_or("missing /resource")?
                == s.spec
                    .application
                    .pointer("/resource")
                    .ok_or("missing /resource")?
            && body.pointer("/start").ok_or("missing /start")? == start
            && body.pointer("/end").ok_or("missing /end")? == end,
        "availability contract differs",
    )?;
    require(
        !body.to_string().contains("Ada") && !body.to_string().contains("Bo"),
        "public response disclosed private names",
    )?;
    body.pointer("/remaining")
        .ok_or("missing /remaining")?
        .as_u64()
        .ok_or_else(|| "remaining capacity absent".into())
}
fn active(s: &Service) -> EvidenceResult<Vec<Value>> {
    let (status, body) = s.request(Method::GET, "/reservations", None, Some(&s.token))?;
    require(
        status == 200 && body.is_array(),
        "private bookings unavailable",
    )?;
    body.as_array()
        .cloned()
        .ok_or_else(|| "bookings absent".into())
}
fn path(item: &Value) -> EvidenceResult<String> {
    let id = item.get("id").ok_or("booking identity absent")?;
    Ok(format!(
        "/reservations/{}",
        id.as_str().map_or_else(|| id.to_string(), str::to_owned)
    ))
}
fn clear(s: &Service) -> EvidenceResult {
    s.clock(s.case.before)?;
    for item in active(s)? {
        require(
            s.request(Method::DELETE, &path(&item)?, None, Some(&s.token))?
                .0
                == 200,
            "fixture cleanup refused",
        )?;
    }
    Ok(())
}
pub(super) fn observe(s: &mut Service, name: &str) -> EvidenceResult {
    let c = s.case;
    let (start, end) = (c.start, c.end);
    match name {
        "public-availability" => {
            clear(s)?;
            require(
                remaining(s, start, end)? == c.capacity
                    && book(s)?.0 == 201
                    && remaining(s, start, end)? == c.capacity - 1,
                "public availability differs",
            )
        }
        "authenticated-mutation" => {
            clear(s)?;
            let before = active(s)?;
            for auth in [None, Some("incorrect")] {
                require(
                    booking(s, "Ada", start, end, 1, auth)?.0 == 401 && active(s)? == before,
                    "unauthorized creation changed bookings",
                )?;
            }
            let (status, item) = book(s)?;
            require(status == 201, "authorized creation refused")?;
            for auth in [None, Some("incorrect")] {
                require(
                    s.request(Method::DELETE, &path(&item)?, None, auth)?.0 == 401
                        && remaining(s, start, end)? == c.capacity - 1,
                    "unauthorized cancellation changed bookings",
                )?;
            }
            require(
                s.request(Method::DELETE, &path(&item)?, None, Some(&s.token))?
                    .0
                    == 200
                    && remaining(s, start, end)? == c.capacity,
                "authorized cancellation refused",
            )
        }
        "capacity-and-intervals" => {
            clear(s)?;
            for (start, end, quantity) in [
                (end, start, 1),
                (start, start, 1),
                ("not-a-date", end, 1),
                (start, end, 0),
            ] {
                require(
                    booking(s, "Ada", start, end, quantity, Some(&s.token))?.0 == 400,
                    "invalid interval or quantity accepted",
                )?;
            }
            require(active(s)?.is_empty(), "initial capacity differs")?;
            fill(s, c.capacity - 1)?;
            let s: &Service = s;
            let mut statuses = std::thread::scope(|scope| {
                let first =
                    scope.spawn(|| booking(s, "Bo", start, end, 1, Some(&s.token)).map(|r| r.0));
                let second =
                    scope.spawn(|| booking(s, "Ci", start, end, 1, Some(&s.token)).map(|r| r.0));
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(vec![
                    first.join().map_err(|_| "contender failed")??,
                    second.join().map_err(|_| "contender failed")??,
                ])
            })?;
            statuses.sort_unstable();
            require(
                statuses == [201, 409] && remaining(s, start, end)? == 0,
                "concurrent bookings exceeded capacity",
            )?;
            require(
                booking(s, "Ada", c.overlap, c.adjacent_end, 1, Some(&s.token))?.0 == 409,
                "overlapping interval exceeded capacity",
            )?;
            let before = active(s)?;
            require(
                book(s)?.0 == 409 && active(s)? == before,
                "over-capacity refusal mutated state",
            )?;
            require(
                booking(s, "Ada", end, c.adjacent_end, 1, Some(&s.token))?.0 == 201
                    && remaining(s, end, c.adjacent_end)? == c.capacity - 1,
                "adjacent intervals overlap",
            )
        }
        "durable-bookings" => {
            clear(s)?;
            fill(s, c.capacity)?;
            require(book(s)?.0 == 409, "initial durable capacity differs")?;
            let rows = active(s)?;
            let item = rows.first().ok_or("booking absent")?;
            require(
                s.request(Method::DELETE, &path(item)?, None, Some(&s.token))?
                    .0
                    == 200
                    && booking(
                        s,
                        "Ada",
                        start,
                        end,
                        booking_quantity(item)?,
                        Some(&s.token),
                    )?
                    .0 == 201,
                "replacement reservation failed",
            )?;
            let before = active(s)?;
            let identity = s.identity.as_ref().ok_or("container absent")?.clone();
            podman(&["stop".into(), "--time=5".into(), identity.clone()])?;
            require(
                s.inspect()?.pointer("/State/Running") == Some(&json!(false)),
                "container stop not observed",
            )?;
            podman(&["rm".into(), identity])?;
            s.identity = None;
            remove_cidfile(&s.spec.directory.join("container.cid"))?;
            s.launch()?;
            s.ready()?;
            require(
                active(s)? == before && remaining(s, start, end)? == 0,
                "bookings changed after container recreation",
            )
        }
        "cancellation-policy" => {
            clear(s)?;
            let (status, item) = book(s)?;
            require(status == 201, "initial booking failed")?;
            let selected = path(&item)?;
            for _ in 0..2 {
                require(
                    s.request(Method::DELETE, &selected, None, Some(&s.token))?
                        .0
                        == 200,
                    "repeated cancellation differs",
                )?;
            }
            require(
                remaining(s, start, end)? == c.capacity,
                "cancellation did not release capacity",
            )?;
            let (status, item) = book(s)?;
            require(status == 201, "second booking failed")?;
            for clock in [c.cutoff, c.end] {
                s.clock(clock)?;
                require(
                    s.request(Method::DELETE, &path(&item)?, None, Some(&s.token))?
                        .0
                        == 409
                        && remaining(s, start, end)? == c.capacity - 1,
                    "cancellation cutoff bypassed",
                )?;
            }
            Ok(())
        }
        "exact-deployment" => {
            let state = s.inspect()?;
            require(
                state
                    .pointer("/Image")
                    .ok_or("missing /Image")?
                    .as_str()
                    .map(|v| v.trim_start_matches("sha256:"))
                    == Some(s.spec.image_identity.as_str())
                    && state.pointer("/HostConfig/ReadonlyRootfs") == Some(&json!(true)),
                "candidate image or root protection differs",
            )?;
            let mounts = state
                .pointer("/Mounts")
                .ok_or("missing /Mounts")?
                .as_array()
                .ok_or("mounts absent")?;
            for (destination, source) in [
                ("/candidate/app", s.spec.candidate.clone()),
                ("/config", s.spec.directory.join("config")),
            ] {
                require(
                    mounts
                        .iter()
                        .filter(|m| {
                            m["Destination"] == destination
                                && m["Source"].as_str() == source.to_str()
                                && m["RW"] == false
                        })
                        .count()
                        == 1,
                    "candidate/config mount differs",
                )?;
            }
            require(
                mounts.iter().all(|m| {
                    m["Destination"].as_str().is_some_and(|v| {
                        [
                            "/candidate/app",
                            "/config",
                            "/data",
                            "/tmp",
                            "/dev/shm",
                            "/etc/hosts",
                            "/etc/hostname",
                            "/etc/resolv.conf",
                        ]
                        .contains(&v)
                    })
                }),
                "unexpected candidate mount",
            )
        }
        _ => Err("unsupported required verifier check".into()),
    }
}

#[cfg(test)]
mod quantity_tests {
    use super::*;

    #[test]
    fn remote_quantity_cannot_truncate_into_a_different_replacement() -> EvidenceResult {
        for quantity in [1, u32::MAX] {
            assert_eq!(booking_quantity(&json!({"quantity":quantity}))?, quantity);
        }
        for value in [
            json!({}),
            json!({"quantity":-1}),
            json!({"quantity":u64::from(u32::MAX) + 1}),
            json!({"quantity":u64::MAX}),
        ] {
            assert!(
                booking_quantity(&value).is_err(),
                "invalid remote quantity accepted: {value}"
            );
        }
        Ok(())
    }
}
