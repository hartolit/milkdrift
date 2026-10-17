//! Daemon naming uses the same authenticated client and durable public command as the GUI.
use super::{ClientError, ControlClient, encoded_path_segment};
use milkdrift_control_protocol::{DaemonMetadataDto, RenameDaemonReceipt, RenameDaemonRequest};
use reqwest::Method;

impl ControlClient {
    /// Reads the current shared name, identity and this actor's rename permission.
    ///
    /// # Errors
    /// Returns transport, protocol, identity mismatch or authorization failures.
    pub async fn daemon_metadata(&self) -> Result<DaemonMetadataDto, ClientError> {
        self.safe_get("v1/daemon/metadata").await
    }

    /// Submits one exact rename without automatic retries.
    ///
    /// On a lost reply, look up the receipt or replay every field of the same request.
    /// A new request identity expresses a new mutation, not recovery.
    ///
    /// # Errors
    /// Returns validation, generation conflict, identity mismatch, authority, capacity or
    /// transport failures. A transport failure does not prove that the rename was refused.
    pub async fn rename_daemon(
        &self,
        request: &RenameDaemonRequest,
    ) -> Result<RenameDaemonReceipt, ClientError> {
        self.json_request(Method::POST, "v1/daemon/rename", Some(request), false)
            .await
    }

    /// Looks up this authenticated actor's exact accepted rename.
    ///
    /// # Errors
    /// Returns invalid identity, missing receipt, current authority or transport failures.
    pub async fn daemon_rename_receipt(
        &self,
        request_id: &str,
    ) -> Result<RenameDaemonReceipt, ClientError> {
        self.safe_get(&format!(
            "v1/daemon/rename-requests/{}",
            encoded_path_segment(request_id)?
        ))
        .await
    }
}

#[cfg(test)]
mod tests {
    use crate::{BearerCredential, ClientConfig, ControlClient};

    #[test]
    fn a_daemon_pin_cannot_be_silently_replaced() -> Result<(), Box<dyn std::error::Error>> {
        let client = ControlClient::new(
            ClientConfig::new("http://127.0.0.1:9734/".parse()?),
            BearerCredential::new("fixture-credential")?,
        )?;
        for invalid in ["", "daemon\r\nforged: yes", "δ", &"x".repeat(257)] {
            assert!(client.clone().with_daemon_identity(invalid).is_err());
        }
        let pinned = client.with_daemon_identity("host:first")?;
        assert!(pinned.clone().with_daemon_identity("host:first").is_ok());
        assert!(pinned.with_daemon_identity("host:replacement").is_err());
        Ok(())
    }
}
