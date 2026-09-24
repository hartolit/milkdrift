use super::{
    Caller, EvidenceResult, Expected, InvocationMode, Qualify, Session, Value, ensure, json,
    number, reference_args, text,
};
use crate::publication;
use std::{thread, time::Duration};
pub(super) struct Link {
    pub(super) internal: String,
    pub(super) public: Option<String>,
    pub(super) outer: Option<Value>,
}
pub(super) fn start(
    s: &mut Session,
    args: &Qualify,
    revision: &Value,
    version: u64,
) -> EvidenceResult<Link> {
    if !args.published {
        s.ok(
            "run-start",
            args![
                "run",
                "start",
                "slotbook-run",
                "slotbook",
                text(&revision["id"])?
            ],
        )?;
        return Ok(Link {
            internal: "slotbook-run".into(),
            public: None,
            outer: None,
        });
    }
    let catalog = s.ok("method-discovery", args!["invocation", "catalog"])?;
    let descriptor = catalog["value"]["catalog"]["entries"]
        .as_array()
        .ok_or("catalog entries absent")?
        .iter()
        .find(|e| e["descriptor"]["identity"] == "managed.slotbook-build.worker")
        .ok_or("worker descriptor absent")?["descriptor"]
        .clone();
    let authority = s.ok("service-authority", args!["daemon", "authority"])?;
    let document = s.write(
        "published-method.json",
        &publication::method(descriptor, revision, &authority["value"]),
    )?;
    s.ok(
        "method-publish",
        args!["method", "publish", document.display()],
    )?;
    for (label, command) in [
        (
            "consumer-cannot-publish",
            args!["method", "publish", document.display()],
        ),
        (
            "consumer-cannot-inspect-method",
            args!["method", "show", "method:slotbook", "--generation", 1],
        ),
        (
            "consumer-cannot-administer-target",
            args![
                "resource",
                "--installation",
                "slotbook-test",
                "--expected-version",
                version,
                "inspect"
            ],
        ),
    ] {
        s.call(label, command, Expected::Refused, Caller::Consumer)?;
    }
    let mut public = None;
    let mut outer = None;
    if args.invocation_mode == InvocationMode::Direct {
        let input = s.write("public-inputs.json", &json!([]))?;
        public = Some(s.invoke(
            "published",
            "method:slotbook",
            "method.invoke",
            &input,
            Caller::Consumer,
        )?);
    } else {
        let mut capability = "method:slotbook".to_owned();
        if args.invocation_mode == InvocationMode::Peer {
            s.call(
                "peer-connect",
                args!["peer", "connect", "host:slotbook-test"],
                Expected::Success,
                Caller::Origin,
            )?;
            let catalog = s.call(
                "origin-catalog",
                args!["capability", "list"],
                Expected::Success,
                Caller::Origin,
            )?;
            let catalog = &catalog["value"];
            let entries = catalog
                .get("items")
                .unwrap_or(catalog)
                .as_array()
                .ok_or("origin catalog absent")?;
            capability = text(
                &entries
                    .iter()
                    .find(|e| e["locality"] == "peer")
                    .ok_or("peer capability absent")?["capability_id"],
            )?
            .into();
        }
        let revision = publication::outer(
            &s.root,
            &s.cli,
            &capability,
            args.invocation_mode == InvocationMode::Peer,
        )?;
        s.call(
            "outer-import",
            args!["blueprint", "import", s.root.join("outer.json").display()],
            Expected::Success,
            Caller::Origin,
        )?;
        s.call(
            "outer-start",
            args![
                "run",
                "start",
                "slotbook-caller",
                "slotbook-caller",
                text(&revision["id"])?
            ],
            Expected::Success,
            Caller::Origin,
        )?;
        outer = Some(revision);
    }
    let mut internal = None;
    for index in 0..100 {
        let page = s.ok(
            &format!("find-internal-{index}"),
            args!["run", "list", "--limit", 32],
        )?;
        let linked = page["value"]["items"]
            .as_array()
            .ok_or("run inventory absent")?
            .iter()
            .filter(|run| {
                let source = &run["published_source"];
                if let Some(execution) = &public {
                    source["execution"] == execution.as_str()
                } else if args.invocation_mode == InvocationMode::Peer {
                    source["type"] == "serving"
                } else {
                    source["run"] == "slotbook-caller"
                }
            })
            .collect::<Vec<_>>();
        if !linked.is_empty() {
            ensure(
                linked.len() == 1,
                "accepted call has multiple internal runs",
            )?;
            internal = Some(text(&linked[0]["run_id"])?.to_owned());
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let internal = internal.ok_or("accepted publication did not create its linked run")?;
    s.call(
        "consumer-cannot-inspect-run",
        args!["run", "show", internal],
        Expected::Refused,
        Caller::Consumer,
    )?;
    let held = s.resource(
        "published-lifetime-hold",
        "inspect",
        0,
        vec![],
        "slotbook-build",
        Expected::Success,
    )?;
    ensure(
        held["blockers"].as_array().is_some_and(|b| !b.is_empty()),
        "publication lacks lifetime hold",
    )?;
    s.resource(
        "busy-remove-refused",
        "remove",
        number(&held["version"])?,
        vec![],
        "slotbook-build",
        Expected::Refused,
    )?;
    let held = s.resource(
        "held-before-reapply",
        "inspect",
        0,
        vec![],
        "slotbook-build",
        Expected::Success,
    )?;
    s.resource(
        "busy-reapply-refused",
        "apply",
        number(&held["version"])?,
        reference_args(&s.worker)?,
        "slotbook-build",
        Expected::Refused,
    )?;
    Ok(Link {
        internal,
        public,
        outer,
    })
}
pub(super) fn complete(s: &Session, args: &Qualify, link: &Link) -> EvidenceResult {
    if args.published && args.invocation_mode != InvocationMode::Direct {
        let outer = s.call(
            "outer-result",
            args!["run", "wait", "slotbook-caller"],
            Expected::Success,
            Caller::Origin,
        )?;
        ensure(
            outer["value"]["terminal"] == "succeeded",
            "outer workflow failed",
        )?;
    }
    if let Some(execution) = &link.public {
        s.call(
            "public-result",
            args!["invocation", "wait", execution],
            Expected::Success,
            Caller::Consumer,
        )?;
        s.call(
            "public-replay",
            args![
                "invocation",
                "submit",
                s.requests
                    .get("published")
                    .ok_or("public request absent")?
                    .display()
            ],
            Expected::Success,
            Caller::Consumer,
        )?;
    }
    Ok(())
}
pub(super) fn replay(s: &Session, link: &Link) -> EvidenceResult {
    if let Some(revision) = &link.outer {
        s.call(
            "outer-start",
            args![
                "run",
                "start",
                "slotbook-caller",
                "slotbook-caller",
                text(&revision["id"])?
            ],
            Expected::Success,
            Caller::Origin,
        )?;
    }
    if link.public.is_some() {
        s.call(
            "public-replay-after-restart",
            args![
                "invocation",
                "submit",
                s.requests
                    .get("published")
                    .ok_or("public request absent")?
                    .display()
            ],
            Expected::Success,
            Caller::Consumer,
        )?;
    }
    Ok(())
}
