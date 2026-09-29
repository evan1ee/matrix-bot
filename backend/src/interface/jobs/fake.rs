//! Scripted `Delivery` for handler tests: never touches Matrix.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::{Delivery, DeliveryError};

#[derive(Default)]
pub(crate) struct FakeDelivery {
    pub connected: AtomicBool,
    /// Popped per send; empty means `Ok(Some("$evt"))`.
    pub room_results: Mutex<VecDeque<Result<Option<String>, DeliveryError>>>,
    /// `(room, plain, html)`
    pub room_sends: Mutex<Vec<(String, String, String)>>,
    /// `(mxid, plain, html)`
    pub dms: Mutex<Vec<(String, String, String)>>,
}

#[expect(dead_code, reason = "first handler tests (Task 8) use it; remove then")]
impl FakeDelivery {
    pub fn connected() -> Arc<Self> {
        let fake = Self::default();
        fake.connected.store(true, Ordering::SeqCst);
        Arc::new(fake)
    }

    pub fn disconnected() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn failing_once(reason: &str) -> Arc<Self> {
        let fake = Self::connected();
        fake.room_results
            .lock()
            .unwrap()
            .push_back(Err(DeliveryError::Failed(reason.to_string())));
        fake
    }
}

#[async_trait]
impl Delivery for FakeDelivery {
    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn send_to_room(
        &self,
        room_id: &str,
        plain: &str,
        html: &str,
    ) -> Result<Option<String>, DeliveryError> {
        if !self.is_connected() {
            return Err(DeliveryError::NotConnected);
        }
        self.room_sends
            .lock()
            .unwrap()
            .push((room_id.into(), plain.into(), html.into()));
        self.room_results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(Some("$evt".to_string())))
    }

    async fn send_dm(&self, mxid: &str, plain: &str, html: &str) -> Result<(), DeliveryError> {
        if !self.is_connected() {
            return Err(DeliveryError::NotConnected);
        }
        self.dms
            .lock()
            .unwrap()
            .push((mxid.into(), plain.into(), html.into()));
        Ok(())
    }
}
