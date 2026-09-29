//! Job handlers: the interface layer of the job runtime. Each handler parses
//! its payload, calls application services and delivers through `Delivery`.
//! No SQL, no business rules.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::de::DeserializeOwned;

use crate::application::AppServices;
use crate::jobs::{JobError, JobHandler};

pub mod briefing;
pub mod reminders;

#[cfg(test)]
pub(crate) mod fake;

/// How long a job waits before retrying when Matrix is disconnected.
pub const MATRIX_RETRY: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    #[error("Matrix is not connected")]
    NotConnected,
    #[error("delivery failed: {0}")]
    Failed(String),
}

/// How handlers reach Matrix; a seam so handler tests never touch a homeserver.
#[async_trait]
pub trait Delivery: Send + Sync {
    fn is_connected(&self) -> bool;
    /// `Ok(Some(event_id))` sent; `Ok(None)` target is not a valid room id
    /// (treated as delivered: frozen behaviour).
    async fn send_to_room(
        &self,
        room_id: &str,
        plain: &str,
        html: &str,
    ) -> Result<Option<String>, DeliveryError>;
    async fn send_dm(&self, mxid: &str, plain: &str, html: &str) -> Result<(), DeliveryError>;
}

/// Every job handler; task 10 registers the rest here.
pub fn handlers(services: &AppServices, delivery: Arc<dyn Delivery>) -> Vec<Arc<dyn JobHandler>> {
    vec![
        Arc::new(reminders::ReminderFireHandler::new(
            services.clone(),
            delivery.clone(),
        )),
        Arc::new(briefing::BriefingSendHandler::new(
            services.clone(),
            delivery,
        )),
    ]
}

/// A payload that does not parse never will: fail the job now.
pub(crate) fn parse_payload<T: DeserializeOwned>(
    payload: serde_json::Value,
) -> Result<T, JobError> {
    serde_json::from_value(payload)
        .map_err(|e| JobError::Permanent(anyhow::anyhow!("malformed payload: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_malformed_payload_is_permanent() {
        let r: Result<crate::domain::jobs::ReminderFirePayload, _> =
            parse_payload(serde_json::json!({"x": 1}));
        assert!(matches!(r, Err(JobError::Permanent(_))));
    }
}
