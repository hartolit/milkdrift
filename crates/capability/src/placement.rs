use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{ContractError, Locality, PeerId};

/// Task-side limits on the machine that may execute an operation.
///
/// An absent (or JSON `null`) dimension accepts any value; an empty set accepts none.
/// Peer IDs are exact authenticated Milkdrift identities, never labels or endpoint URLs.
/// A peer set also requires peer locality. These constraints intersect all other task
/// requirements and authority; they never authorize a host on their own.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementRequirement {
    #[serde(skip_serializing_if = "Option::is_none")]
    localities: Option<BTreeSet<Locality>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    peers: Option<BTreeSet<PeerId>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlacementWire {
    localities: Option<Vec<Locality>>,
    peers: Option<Vec<PeerId>>,
}

milkdrift_contracts::deserialize_via!(PlacementRequirement, PlacementWire, |wire| {
    (|| Self::new(exact_set(wire.localities, 4)?, exact_set(wire.peers, 128)?))()
});

fn exact_set<T: Ord>(
    values: Option<Vec<T>>,
    maximum: usize,
) -> Result<Option<BTreeSet<T>>, ContractError> {
    let Some(values) = values else {
        return Ok(None);
    };
    if values.len() > maximum || values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(ContractError::InvalidContract(
            "placement sets must be bounded, unique, and in canonical order".to_owned(),
        ));
    }
    Ok(Some(values.into_iter().collect()))
}

impl PlacementRequirement {
    /// Constructs intersecting locality and peer allowlists (at most 128 peers).
    /// `None` is unrestricted; `Some(empty)` denies every placement. Nonempty peers
    /// combined with a locality set that excludes `Peer` are contradictory and refused.
    pub fn new(
        localities: Option<BTreeSet<Locality>>,
        peers: Option<BTreeSet<PeerId>>,
    ) -> Result<Self, ContractError> {
        if peers.as_ref().is_some_and(|peers| peers.len() > 128) {
            return Err(ContractError::InvalidContract(
                "placement permits at most 128 exact peers".to_owned(),
            ));
        }
        if peers.as_ref().is_some_and(|peers| !peers.is_empty())
            && localities
                .as_ref()
                .is_some_and(|values| !values.contains(&Locality::Peer))
        {
            return Err(ContractError::InvalidContract(
                "required peers contradict the permitted localities".to_owned(),
            ));
        }
        Ok(Self { localities, peers })
    }

    /// Permitted localities; absence is unrestricted before intersecting the peer set.
    #[must_use]
    pub const fn localities(&self) -> Option<&BTreeSet<Locality>> {
        self.localities.as_ref()
    }

    /// Permitted authenticated peers; presence also requires peer locality.
    #[must_use]
    pub const fn peers(&self) -> Option<&BTreeSet<PeerId>> {
        self.peers.as_ref()
    }

    /// Whether an explicit empty set makes every candidate ineligible.
    #[must_use]
    pub fn denies_all(&self) -> bool {
        self.localities.as_ref().is_some_and(BTreeSet::is_empty)
            || self.peers.as_ref().is_some_and(BTreeSet::is_empty)
    }

    pub(crate) fn matches(&self, locality: Locality, peer: Option<&PeerId>) -> bool {
        self.localities
            .as_ref()
            .is_none_or(|values| values.contains(&locality))
            && self.peers.as_ref().is_none_or(|values| {
                locality == Locality::Peer && peer.is_some_and(|peer| values.contains(peer))
            })
    }
}
