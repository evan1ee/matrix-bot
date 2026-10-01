//! `ChannelAdapter::send` for Matrix: neutral `OutgoingMessage`s onto the
//! homeserver, with every matrix-sdk failure mapped to a `ChannelError` at this
//! boundary so no caller sees the SDK's error types.

use matrix_sdk::room::Room;
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use matrix_sdk::ruma::{RoomId, UserId};
use matrix_sdk::Client;
use tracing::{error, warn};

use crate::domain::channels::{Channel, ConversationAddress};
use crate::interface::channels::adapter::{ChannelError, SentRef};
use crate::interface::channels::matrix::live_client::MatrixHandle;
use crate::interface::channels::matrix::voice_upload;
use crate::interface::channels::message::{
    AttachmentKind, Content, MessageRef, OutgoingMessage, Target,
};

/// Where a message goes, validated before any client is needed.
enum Destination<'a> {
    Room(&'a RoomId),
    Direct(&'a UserId),
}

fn destination(target: &Target) -> Result<Destination<'_>, ChannelError> {
    match target {
        Target::Conversation(ConversationAddress::Matrix(room)) => {
            <&RoomId>::try_from(room.as_str())
                .map(Destination::Room)
                .map_err(|_| ChannelError::Rejected(format!("not a Matrix room id: {room}")))
        }
        Target::Direct(identity) if identity.channel() == Channel::Matrix => {
            <&UserId>::try_from(identity.user_id().as_str())
                .map(Destination::Direct)
                .map_err(|_| {
                    ChannelError::Rejected(format!(
                        "not a Matrix user id: {}",
                        identity.user_id().as_str()
                    ))
                })
        }
        other => Err(ChannelError::Rejected(format!(
            "not a Matrix target: {other:?}"
        ))),
    }
}

/// The room message for `content`: rich when it has HTML, else plain.
fn text_content(content: &Content) -> RoomMessageEventContent {
    match &content.html {
        Some(html) => RoomMessageEventContent::text_html(&content.plain, html),
        None => RoomMessageEventContent::text_plain(&content.plain),
    }
}

pub async fn send(
    handle: &MatrixHandle,
    message: OutgoingMessage,
) -> Result<SentRef, ChannelError> {
    // Shape errors first: a message that can never be delivered is `Rejected`
    // whether or not the connection is up, so a retry loop does not wait on it.
    let destination = destination(&message.target)?;
    if let Some(unsupported) = message
        .content
        .attachments
        .iter()
        .find(|a| a.kind != AttachmentKind::Audio)
    {
        return Err(ChannelError::Rejected(format!(
            "Matrix delivery has no path for a {:?} attachment",
            unsupported.kind
        )));
    }
    let client = handle
        .current()
        .ok_or(ChannelError::NotConnected(Channel::Matrix))?;

    match destination {
        Destination::Room(room_id) => deliver_to_room_id(&client, room_id, &message.content).await,
        Destination::Direct(user_id) => {
            let room = dm_room(&client, user_id).await?;
            deliver(&room, &message.content).await
        }
    }
}

/// Send into a room the bot already has; when that fails (or the room is not
/// in the store) join it and send once more, the way the job handlers always did.
async fn deliver_to_room_id(
    client: &Client,
    room_id: &RoomId,
    content: &Content,
) -> Result<SentRef, ChannelError> {
    if let Some(room) = client.get_room(room_id) {
        match deliver(&room, content).await {
            Ok(sent) => return Ok(sent),
            Err(err) => warn!(%room_id, error = %err, "Send failed; retrying via join"),
        }
    } else {
        warn!(%room_id, "No room handle in the store; joining");
    }
    let joined = client
        .join_room_by_id(room_id)
        .await
        .map_err(|e| ChannelError::Transient(format!("join {room_id}: {e}")))?;
    deliver(&joined, content).await
}

/// Audio first (voice bubbles), then the text: the order a voice reply has
/// always had in the room.
async fn deliver(room: &Room, content: &Content) -> Result<SentRef, ChannelError> {
    for attachment in &content.attachments {
        voice_upload::send_voice_message(room, attachment)
            .await
            .map_err(ChannelError::Transient)?;
    }
    if content.plain.is_empty() && content.html.is_none() && !content.attachments.is_empty() {
        return Ok(SentRef { message: None });
    }
    let response = room
        .send(text_content(content))
        .await
        .map_err(|e| ChannelError::Transient(format!("send to {}: {e}", room.room_id())))?;
    Ok(SentRef {
        message: Some(MessageRef(response.response.event_id.to_string())),
    })
}

/// The user's DM with the bot: an existing one first (creating a second each
/// time would litter their client), else a new one — a notice such as a
/// recovery warning must not be dropped for want of a room.
async fn dm_room(client: &Client, user_id: &UserId) -> Result<Room, ChannelError> {
    for room in client.joined_rooms() {
        if !room.is_direct().await.unwrap_or(false) {
            continue;
        }
        if matches!(room.get_member(user_id).await, Ok(Some(_))) {
            return Ok(room);
        }
    }
    client
        .create_dm(user_id)
        .await
        .map_err(|e| ChannelError::Transient(format!("open a DM with {user_id}: {e}")))
}

/// Sends an HTML message to a user's DM and logs a failure instead of
/// returning it: the job handlers' `Delivery::send_dm` contract.
// Plan A Task 8 deletes this with the `Delivery` trait: job handlers then send through the registry.
pub async fn send_dm_html(client: &Client, mxid: &str, plain: &str, html: &str) {
    let Ok(user_id) = <&UserId>::try_from(mxid) else {
        error!("Cannot notify {}: not a valid user ID", mxid);
        return;
    };
    let content = Content {
        plain: plain.to_owned(),
        html: Some(html.to_owned()),
        ..Content::default()
    };
    let result = match dm_room(client, user_id).await {
        Ok(room) => deliver(&room, &content).await,
        Err(err) => Err(err),
    };
    if let Err(err) = result {
        error!("Could not send an HTML notification to {}: {}", mxid, err);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channels::PlatformIdentity;
    use crate::interface::channels::message::Reply;

    #[test]
    fn a_matrix_room_and_a_matrix_identity_are_destinations() {
        let room = Target::Conversation(ConversationAddress::Matrix("!r:x".into()));
        assert!(matches!(destination(&room), Ok(Destination::Room(_))));
        let user = Target::Direct(PlatformIdentity::matrix("@u:x").unwrap());
        assert!(matches!(destination(&user), Ok(Destination::Direct(_))));
    }

    #[test]
    fn anything_else_is_rejected() {
        for target in [
            Target::Conversation(ConversationAddress::Telegram(1)),
            Target::Conversation(ConversationAddress::Web),
            Target::Conversation(ConversationAddress::Matrix("no-sigil".into())),
        ] {
            assert!(matches!(
                destination(&target),
                Err(ChannelError::Rejected(_))
            ));
        }
    }

    #[test]
    fn html_content_is_sent_rich_and_the_rest_plain() {
        let rich = text_content(&Reply::html("<b>x</b>").content);
        assert_eq!(rich.body(), "x");
        assert!(matches!(
            rich.msgtype,
            matrix_sdk::ruma::events::room::message::MessageType::Text(ref t) if t.formatted.is_some()
        ));
        let plain = text_content(&Content::text("hello"));
        assert!(matches!(
            plain.msgtype,
            matrix_sdk::ruma::events::room::message::MessageType::Text(ref t) if t.formatted.is_none()
        ));
    }
}
