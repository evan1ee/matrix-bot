//! Email outbound formatting and SMTP transport via lettre.
//! Pure rendering and error mapping are fully covered by unit tests without network calls.

use std::time::Duration;

use lettre::message::header::{self, ContentDisposition, ContentType, InReplyTo, References};
use lettre::message::{Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::config::EmailConfig;
use crate::domain::channels::{Channel, ConversationAddress};
use crate::interface::channels::adapter::{ChannelError, SentRef};
use crate::interface::channels::message::{Action, Content, MessageRef, OutgoingMessage, Target};
use crate::utils::html::html_escape;

/// Timeout for SMTP operations.
pub const SMTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Extracts the recipient email address from an `OutgoingMessage` target.
pub fn extract_recipient(target: &Target) -> Result<String, ChannelError> {
    match target {
        Target::Conversation(ConversationAddress::Email(addr)) => Ok(addr.clone()),
        Target::Direct(identity) if identity.channel() == Channel::Email => {
            Ok(identity.user_id().as_str().to_owned())
        }
        other => Err(ChannelError::Rejected(format!(
            "target {other:?} is not an email destination"
        ))),
    }
}

/// Derives an email subject line.
///
/// Replies preserve or prepend `Re:`. Unsolicited messages take the first line
/// of plain text (up to 80 chars) or fallback to "Matrix Bot".
pub fn derive_subject(content: &Content, reply_to_subject: Option<&str>) -> String {
    if let Some(subject) = reply_to_subject {
        let trimmed = subject.trim();
        if trimmed.to_ascii_lowercase().starts_with("re:") {
            trimmed.to_owned()
        } else {
            format!("Re: {trimmed}")
        }
    } else {
        let first_line = content
            .plain
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("Matrix Bot");
        if first_line.chars().count() > 80 {
            let truncated: String = first_line.chars().take(77).collect();
            format!("{truncated}...")
        } else {
            first_line.to_owned()
        }
    }
}

/// Formats interactive actions as plain text list.
pub fn format_actions_plain(actions: &[Action]) -> String {
    if actions.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\nReply to this email with:\n");
    for action in actions {
        out.push_str(&format!("• {}\n", action.command));
    }
    out
}

/// Formats interactive actions as HTML list.
pub fn format_actions_html(actions: &[Action]) -> String {
    if actions.is_empty() {
        return String::new();
    }
    let mut out = String::from("<br/><br/><p>Reply to this email with:</p><ul>");
    for action in actions {
        out.push_str(&format!(
            "<li><code>{}</code></li>",
            html_escape(&action.command)
        ));
    }
    out.push_str("</ul>");
    out
}

/// Builds a `lettre::Message` with multipart/alternative (plain + html) and optional attachments.
pub fn render_email(
    from: &Mailbox,
    to: &Mailbox,
    subject: &str,
    content: &Content,
    in_reply_to: Option<&str>,
    references: Option<&str>,
) -> Result<Message, ChannelError> {
    let plain_body = format!(
        "{}{}",
        content.plain,
        format_actions_plain(&content.actions)
    );
    let html_body = match &content.html {
        Some(html) => format!("{}{}", html, format_actions_html(&content.actions)),
        None => format!(
            "<p>{}</p>{}",
            html_escape(&content.plain),
            format_actions_html(&content.actions)
        ),
    };

    let alt_part = MultiPart::alternative_plain_html(plain_body, html_body);

    let body_part = if content.attachments.is_empty() {
        alt_part
    } else {
        let mut mixed = MultiPart::mixed().multipart(alt_part);
        for att in &content.attachments {
            let content_type = ContentType::parse(&att.mime)
                .unwrap_or_else(|_| ContentType::parse("application/octet-stream").unwrap());
            let disposition = ContentDisposition::attachment(&att.filename);
            let part = SinglePart::builder()
                .header(content_type)
                .header(disposition)
                .body(att.bytes.clone());
            mixed = mixed.singlepart(part);
        }
        mixed
    };

    let mut builder = Message::builder()
        .from(from.clone())
        .to(to.clone())
        .subject(subject);

    if let Some(msg_id) = in_reply_to {
        let formatted = format!("<{}>", msg_id.trim_matches(['<', '>']));
        builder = builder.header(InReplyTo::from(formatted));
    }

    if let Some(refs) = references {
        let formatted = format!("<{}>", refs.trim_matches(['<', '>']));
        builder = builder.header(References::from(formatted));
    }

    builder
        .multipart(body_part)
        .map_err(|e| ChannelError::Rejected(format!("failed to build MIME message: {e}")))
}

/// Maps lettre SMTP errors to `ChannelError::Transient` or `ChannelError::Rejected`.
pub fn map_smtp_error(err: lettre::transport::smtp::Error) -> ChannelError {
    if err.is_permanent() {
        ChannelError::Rejected(err.to_string())
    } else {
        ChannelError::Transient(err.to_string())
    }
}

/// SMTP client sending emails via `AsyncSmtpTransport`.
#[derive(Clone)]
pub struct EmailSender {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from_mailbox: Mailbox,
}

impl EmailSender {
    /// Creates a new `EmailSender` from `EmailConfig`.
    pub fn new(config: &EmailConfig) -> anyhow::Result<Self> {
        let from_mailbox: Mailbox = config
            .from
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid EMAIL_FROM mailbox format: {e}"))?;

        let host = config
            .smtp_url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("EMAIL_SMTP_URL must specify a host"))?;

        let scheme = config.smtp_url.scheme();
        let builder = if scheme == "smtps" {
            let port = config.smtp_url.port().unwrap_or(465);
            AsyncSmtpTransport::<Tokio1Executor>::relay(host)?.port(port)
        } else {
            let port = config.smtp_url.port().unwrap_or(587);
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)?.port(port)
        };

        let transport = builder
            .credentials(Credentials::new(
                config.username.clone(),
                config.password.clone(),
            ))
            .timeout(Some(SMTP_TIMEOUT))
            .build();

        Ok(Self {
            transport,
            from_mailbox,
        })
    }

    pub fn from_mailbox(&self) -> &Mailbox {
        &self.from_mailbox
    }

    /// Sends an outgoing message over SMTP.
    pub async fn send_message(
        &self,
        outgoing: OutgoingMessage,
        reply_to_subject: Option<&str>,
    ) -> Result<SentRef, ChannelError> {
        let recipient_str = extract_recipient(&outgoing.target)?;
        let to_mailbox: Mailbox = recipient_str
            .parse()
            .map_err(|e| ChannelError::Rejected(format!("invalid recipient address: {e}")))?;

        let subject = derive_subject(&outgoing.content, reply_to_subject);
        let in_reply_to = outgoing.reply_to.as_ref().map(|r| r.0.as_str());

        let email = render_email(
            &self.from_mailbox,
            &to_mailbox,
            &subject,
            &outgoing.content,
            in_reply_to,
            in_reply_to,
        )?;

        let sent_id = email
            .headers()
            .get::<header::MessageId>()
            .map(|id| MessageRef(id.as_ref().to_owned()));

        self.transport.send(email).await.map_err(map_smtp_error)?;

        Ok(SentRef { message: sent_id })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channels::{PlatformIdentity, PlatformUserId};
    use crate::interface::channels::message::{Attachment, AttachmentKind, Reply};

    #[test]
    fn test_derive_subject_replies() {
        let content = Content::text("some text");
        assert_eq!(
            derive_subject(&content, Some("Re: Task list")),
            "Re: Task list"
        );
        assert_eq!(derive_subject(&content, Some("re: status")), "re: status");
        assert_eq!(derive_subject(&content, Some("Task list")), "Re: Task list");
    }

    #[test]
    fn test_derive_subject_unsolicited() {
        let content = Content::text("First line of message\nSecond line");
        assert_eq!(derive_subject(&content, None), "First line of message");

        let empty = Content::text("   \n  \n");
        assert_eq!(derive_subject(&empty, None), "Matrix Bot");

        let long = Content::text("a".repeat(100));
        assert_eq!(
            derive_subject(&long, None),
            format!("{}...", "a".repeat(77))
        );
    }

    #[test]
    fn test_format_actions() {
        let actions = vec![
            Action {
                label: "Add Milk".into(),
                command: "/todo add Milk".into(),
            },
            Action {
                label: "Done".into(),
                command: "/todo done 1".into(),
            },
        ];
        let plain = format_actions_plain(&actions);
        assert!(plain.contains("Reply to this email with:"));
        assert!(plain.contains("• /todo add Milk"));
        assert!(plain.contains("• /todo done 1"));

        let html = format_actions_html(&actions);
        assert!(html.contains("<code>/todo add Milk</code>"));
        assert!(html.contains("<code>/todo done 1</code>"));
    }

    #[test]
    fn test_extract_recipient() {
        let conv = Target::Conversation(ConversationAddress::Email("user@example.com".into()));
        assert_eq!(extract_recipient(&conv).unwrap(), "user@example.com");

        let user_id = PlatformUserId::parse(Channel::Email, "user@example.com").unwrap();
        let direct = Target::Direct(PlatformIdentity::new(Channel::Email, user_id, None));
        assert_eq!(extract_recipient(&direct).unwrap(), "user@example.com");

        let matrix_conv = Target::Conversation(ConversationAddress::Matrix("!room:server".into()));
        assert!(extract_recipient(&matrix_conv).is_err());
    }

    #[test]
    fn test_render_email_mime() {
        let from: Mailbox = "Bot <bot@example.com>".parse().unwrap();
        let to: Mailbox = "User <user@example.com>".parse().unwrap();
        let reply = Reply::html("<b>Your tasks:</b><br/>1. Buy milk").with_actions(vec![Action {
            label: "Done".into(),
            command: "/todo done 1".into(),
        }]);

        let email = render_email(
            &from,
            &to,
            "Re: Status",
            &reply.content,
            Some("orig-msg-123@example.com"),
            Some("orig-msg-123@example.com"),
        )
        .expect("should render email");

        let raw = String::from_utf8(email.formatted()).expect("valid utf8");
        assert!(raw.contains("From: Bot <bot@example.com>"));
        assert!(raw.contains("To: User <user@example.com>"));
        assert!(raw.contains("Subject: Re: Status"));
        assert!(raw.contains("In-Reply-To: <orig-msg-123@example.com>"));
        assert!(raw.contains("References: <orig-msg-123@example.com>"));
        assert!(raw.contains("multipart/alternative"));
        assert!(raw.contains("text/plain"));
        assert!(raw.contains("text/html"));
        assert!(raw.contains("Reply to this email with:"));
        assert!(raw.contains("/todo done 1"));
    }

    #[test]
    fn test_render_email_with_attachments() {
        let from: Mailbox = "bot@example.com".parse().unwrap();
        let to: Mailbox = "user@example.com".parse().unwrap();
        let mut content = Content::text("Here is your photo");
        content.attachments.push(Attachment {
            kind: AttachmentKind::Image,
            mime: "image/png".into(),
            filename: "photo.png".into(),
            bytes: vec![1, 2, 3, 4],
            duration_ms: None,
            waveform: None,
        });

        let email = render_email(&from, &to, "Photo", &content, None, None)
            .expect("should render email with attachment");

        let raw = String::from_utf8(email.formatted()).expect("valid utf8");
        assert!(raw.contains("multipart/mixed"));
        assert!(raw.contains("image/png"));
        assert!(raw.contains("photo.png"));
    }

    #[test]
    fn test_sender_construction_smtps_and_smtp() {
        use url::Url;

        let smtps_cfg = EmailConfig {
            imap_url: Url::parse("imaps://imap.example.com").unwrap(),
            smtp_url: Url::parse("smtps://smtp.example.com:465").unwrap(),
            username: "user".into(),
            password: "password".into(),
            from: "bot@example.com".into(),
            authserv_id: "mx.example.com".into(),
            poll_seconds: 60,
            processed_folder: None,
        };
        assert!(EmailSender::new(&smtps_cfg).is_ok());

        let smtp_cfg = EmailConfig {
            imap_url: Url::parse("imaps://imap.example.com").unwrap(),
            smtp_url: Url::parse("smtp://smtp.example.com:587").unwrap(),
            username: "user".into(),
            password: "password".into(),
            from: "bot@example.com".into(),
            authserv_id: "mx.example.com".into(),
            poll_seconds: 60,
            processed_folder: None,
        };
        assert!(EmailSender::new(&smtp_cfg).is_ok());
    }
}
