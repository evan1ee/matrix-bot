//! Sending a reply into a Matrix room.
//!
//! The same four steps — strip the HTML for the plain-text fallback, build the
//! message, send it, log a failure rather than propagating one — were written
//! out at every call site. A failure to send is logged and swallowed on purpose:
//! by the time we are replying, whatever the reply describes has already
//! happened, so returning an error would not undo it.

use matrix_sdk::room::Room;
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use matrix_sdk::ruma::OwnedEventId;
use tracing::error;

/// Sends an HTML reply, returning the event id when it was accepted.
pub async fn send_html(room: &Room, html: &str) -> Option<OwnedEventId> {
    let plain = crate::utils::strip_html(html);
    let content = RoomMessageEventContent::text_html(&plain, html);
    match room.send(content).await {
        Ok(resp) => Some(resp.response.event_id),
        Err(err) => {
            error!("Failed sending reply to room {}: {:?}", room.room_id(), err);
            None
        }
    }
}
