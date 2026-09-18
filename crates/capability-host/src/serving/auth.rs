use milkdrift_capability::PeerId;
/// Server authentication boundary mapping current credential bytes to configured identity.
pub trait PeerAuthenticator: Send + Sync {
    /// Authenticates at a boundary time. Payload claims cannot influence the result.
    fn authenticate(&self, supplied: &[u8], now_unix_ms: u64) -> Option<PeerId>;
}
