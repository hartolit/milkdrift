use super::{
    require,
    service::{Service, podman, remove_cidfile},
};
use milkdrift_evidence::EvidenceResult;
use reqwest::Method;
use serde_json::{Value, json};

const START: &str = "2027-04-10T10:00:00Z";
const END: &str = "2027-04-10T11:00:00Z";
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
    booking(s, "Ada", START, END, 1, Some(&s.token))
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
            && body["resource"] == s.spec.application["resource"]
            && body["start"] == start
            && body["end"] == end,
        "availability contract differs",
    )?;
    require(
        !body.to_string().contains("Ada") && !body.to_string().contains("Bo"),
        "public response disclosed private names",
    )?;
    body["remaining"]
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
        id.as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| id.to_string())
    ))
}
fn clear(s: &Service) -> EvidenceResult {
    s.clock("2027-04-10T09:00:00Z")?;
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
    match name {
        "public-availability" => {
            clear(s)?;
            require(
                remaining(s, START, END)? == 2
                    && book(s)?.0 == 201
                    && remaining(s, START, END)? == 1,
                "public availability differs",
            )
        }
        "authenticated-mutation" => {
            clear(s)?;
            let before = active(s)?;
            for auth in [None, Some("incorrect")] {
                require(
                    booking(s, "Ada", START, END, 1, auth)?.0 == 401 && active(s)? == before,
                    "unauthorized creation changed bookings",
                )?;
            }
            let (status, item) = book(s)?;
            require(status == 201, "authorized creation refused")?;
            for auth in [None, Some("incorrect")] {
                require(
                    s.request(Method::DELETE, &path(&item)?, None, auth)?.0 == 401
                        && remaining(s, START, END)? == 1,
                    "unauthorized cancellation changed bookings",
                )?;
            }
            require(
                s.request(Method::DELETE, &path(&item)?, None, Some(&s.token))?
                    .0
                    == 200
                    && remaining(s, START, END)? == 2,
                "authorized cancellation refused",
            )
        }
        "capacity-and-intervals" => {
            clear(s)?;
            for (start, end, quantity) in [
                (END, START, 1),
                (START, START, 1),
                ("not-a-date", END, 1),
                (START, END, 0),
            ] {
                require(
                    booking(s, "Ada", start, end, quantity, Some(&s.token))?.0 == 400,
                    "invalid interval or quantity accepted",
                )?;
            }
            require(
                active(s)?.is_empty() && book(s)?.0 == 201,
                "initial capacity differs",
            )?;
            let s: &Service = s;
            let mut statuses = std::thread::scope(|scope| {
                let first =
                    scope.spawn(|| booking(s, "Bo", START, END, 1, Some(&s.token)).map(|r| r.0));
                let second =
                    scope.spawn(|| booking(s, "Ci", START, END, 1, Some(&s.token)).map(|r| r.0));
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(vec![
                    first.join().map_err(|_| "contender failed")??,
                    second.join().map_err(|_| "contender failed")??,
                ])
            })?;
            statuses.sort_unstable();
            require(
                statuses == [201, 409] && remaining(s, START, END)? == 0 && active(s)?.len() == 2,
                "concurrent bookings exceeded capacity",
            )?;
            let before = active(s)?;
            require(
                book(s)?.0 == 409 && active(s)? == before,
                "over-capacity refusal mutated state",
            )?;
            require(
                booking(s, "Ada", END, "2027-04-10T12:00:00Z", 1, Some(&s.token))?.0 == 201
                    && remaining(s, END, "2027-04-10T12:00:00Z")? == 1,
                "adjacent intervals overlap",
            )
        }
        "durable-bookings" => {
            clear(s)?;
            require(
                book(s)?.0 == 201
                    && booking(s, "Bo", START, END, 1, Some(&s.token))?.0 == 201
                    && book(s)?.0 == 409,
                "initial durable capacity differs",
            )?;
            let rows = active(s)?;
            let item = rows.first().ok_or("booking absent")?;
            require(
                s.request(Method::DELETE, &path(item)?, None, Some(&s.token))?
                    .0
                    == 200
                    && book(s)?.0 == 201,
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
                active(s)? == before && remaining(s, START, END)? == 0,
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
                remaining(s, START, END)? == 2,
                "cancellation did not release capacity",
            )?;
            let (status, item) = book(s)?;
            require(status == 201, "second booking failed")?;
            for clock in [START, "2027-04-10T10:30:00Z"] {
                s.clock(clock)?;
                require(
                    s.request(Method::DELETE, &path(&item)?, None, Some(&s.token))?
                        .0
                        == 409
                        && remaining(s, START, END)? == 1,
                    "cancellation cutoff bypassed",
                )?;
            }
            Ok(())
        }
        "exact-deployment" => {
            let state = s.inspect()?;
            require(
                state["Image"]
                    .as_str()
                    .map(|v| v.trim_start_matches("sha256:"))
                    == Some(s.spec.image_identity.as_str())
                    && state.pointer("/HostConfig/ReadonlyRootfs") == Some(&json!(true)),
                "candidate image or root protection differs",
            )?;
            let mounts = state["Mounts"].as_array().ok_or("mounts absent")?;
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
