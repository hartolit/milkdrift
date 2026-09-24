//! Bounded publication inventory and journal-derived link integrity.
use super::{ScanContext, phase};
use crate::{
    error, json,
    schema::{
        PUBLISHED_LOCAL_LINKS, PUBLISHED_LOCAL_PENDING, PUBLISHED_METHOD_HEADS, PUBLISHED_METHODS,
    },
};
use milkdrift_persistence::{
    PersistenceError,
    published::{PublishedInvocationSource, PublishedMethodRecord},
};

pub(super) fn scan(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    let methods = read.open_table(PUBLISHED_METHODS).map_err(error::redb)?;
    let heads = read
        .open_table(PUBLISHED_METHOD_HEADS)
        .map_err(error::redb)?;
    let links = read
        .open_table(PUBLISHED_LOCAL_LINKS)
        .map_err(error::redb)?;
    let pending = read
        .open_table(PUBLISHED_LOCAL_PENDING)
        .map_err(error::redb)?;
    context.string_bytes(
        phase::PUBLISHED_METHODS,
        &methods,
        "published_methods",
        |key, bytes| {
            let record: PublishedMethodRecord = json::decode(bytes, "published method")?;
            record.method.validate()?;
            let id = record.method.descriptor.identity();
            let generation = record.method.descriptor.descriptor_revision();
            if key != format!("{id}/{generation:020}")
                || record.version != if record.retired { 2 } else { 1 }
                || record.retired != record.retirement_request.is_some()
                || !record.authorization.is_allowed()
                || heads
                    .get(id.as_str())
                    .map_err(error::redb)?
                    .is_none_or(|head| head.value() < generation)
            {
                return Err(error::corruption(
                    "published generation, state, authority or head differs",
                ));
            }
            if generation > 1
                && methods
                    .get(format!("{id}/{:020}", generation - 1).as_str())
                    .map_err(error::redb)?
                    .is_none()
            {
                return Err(error::corruption(
                    "published predecessor generation is absent",
                ));
            }
            Ok(())
        },
    )?;
    context.string_u64(
        phase::PUBLISHED_HEADS,
        &heads,
        "published_heads",
        |id, generation| {
            if methods
                .get(format!("{id}/{generation:020}").as_str())
                .map_err(error::redb)?
                .is_none()
                || generation == 0
            {
                return Err(error::corruption("published head lost its generation"));
            }
            Ok(())
        },
    )?;
    context.string_u64(
        phase::PUBLISHED_LINKS,
        &links,
        "published_links",
        |key, sequence| {
            let source: PublishedInvocationSource =
                serde_json::from_str(key).map_err(|error| error::corruption(error.to_string()))?;
            let plan = crate::published::association_read(read, &source)?
                .ok_or_else(|| error::corruption("publication link lost its journal event"))?;
            if sequence == 0
                || serde_json::to_string(&source)
                    .map_err(|error| error::corruption(error.to_string()))?
                    != key
            {
                return Err(error::corruption("noncanonical publication link"));
            }
            let bytes = methods
                .get(format!("{}/{:020}", plan.capability, plan.generation).as_str())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("accepted publication lost its method"))?;
            let record: PublishedMethodRecord = json::decode(bytes.value(), "published method")?;
            if record.method.digest()? != plan.method_digest
                || record.method.allowance != *plan.allowance.budget()
            {
                return Err(error::corruption(
                    "accepted publication implementation or allowance differs",
                ));
            }
            Ok(())
        },
    )?;
    context.string_u64(
        phase::PUBLISHED_PENDING,
        &pending,
        "published_pending",
        |key, sequence| {
            if links
                .get(key)
                .map_err(error::redb)?
                .is_none_or(|link| link.value() != sequence)
            {
                return Err(error::corruption(
                    "pending publication differs from authoritative link",
                ));
            }
            Ok(())
        },
    )
}
