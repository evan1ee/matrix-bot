//! Direct-message delivery shared by the alert handlers (portfolio price
//! alerts, budget alerts): who is reachable right now, and the DM fan-out.

use tracing::warn;

use super::{message, Outbound};
use crate::application::{AppError, AppServices};
use crate::domain::accounts::AccountId;
use crate::domain::channels::{LinkedIdentity, PlatformIdentity, PlatformUserId};
use crate::interface::channels::message::Target;

/// Whether `owner` can receive an alert right now: either they have at
/// least one linked identity on a connected channel, or they have no
/// linked identities on any channel configured on this deployment (web-only).
pub(crate) async fn owner_ready(
    services: &AppServices,
    outbound: &dyn Outbound,
    owner: &AccountId,
) -> Result<bool, AppError> {
    let identities = services.identity.list(owner).await?;
    let configured: Vec<_> = identities
        .into_iter()
        .filter(|id| outbound.is_configured(id.channel))
        .collect();
    Ok(configured.is_empty()
        || configured
            .iter()
            .any(|id| outbound.is_connected(id.channel)))
}

/// DMs every linked identity on a connected channel; a failed send is logged
/// and skipped, so one dead identity never blocks the others.
pub(crate) async fn dm_linked_identities(
    outbound: &dyn Outbound,
    identities: Vec<LinkedIdentity>,
    plain: &str,
    html: &str,
) {
    for linked in identities {
        if !outbound.is_configured(linked.channel) || !outbound.is_connected(linked.channel) {
            continue;
        }
        let user_id = match PlatformUserId::parse(linked.channel, &linked.user_id) {
            Ok(id) => id,
            Err(err) => {
                warn!(channel = %linked.channel, user_id = %linked.user_id, error = %err, "Alert DM not sent");
                continue;
            }
        };
        let identity = PlatformIdentity::new(linked.channel, user_id, linked.display_name);
        if let Err(err) = outbound
            .send(message(Target::Direct(identity), plain, html))
            .await
        {
            warn!(channel = %linked.channel, user_id = %linked.user_id, error = %err, "Alert DM not sent");
        }
    }
}
