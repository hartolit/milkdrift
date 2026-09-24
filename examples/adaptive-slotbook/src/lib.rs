//! The finite Slotbook example application, deployed as one immutable native executable.
//! The separate seeded binary reproduces two explicit fixture defects; it is not a model result.
mod bookings;
mod http;

/// The two independently built candidate artifacts used by the deterministic qualification.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Candidate {
    /// Enforce mutation authentication and persist accepted bookings.
    Corrected,
    /// Deliberately violate authentication and durability for the repair scenario.
    SeededFixture,
}

/// Serve the selected candidate using the protected container's fixed configuration/data mounts.
/// Invalid configuration or retained data refuses startup before the listener is bound.
pub async fn serve(candidate: Candidate) -> Result<(), Box<dyn std::error::Error>> {
    http::serve(candidate).await
}
