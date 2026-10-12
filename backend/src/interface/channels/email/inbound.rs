//! Email inbound normalization: MIME parsing, DMARC verification,
//! auto-reply detection, quote/signature stripping, and command parsing.
//! Pure: no network or services, tested against RFC 5322 fixtures alone.

use mail_parser::{Message, MessageParser};

use crate::domain::channels::{Channel, ConversationAddress, PlatformIdentity, PlatformUserId};
use crate::interface::channels::inbound::InboundText;
use crate::interface::channels::message::{IncomingMessage, MessageRef};
use crate::utils::strip_html;

/// Built-in command keywords recognised without a leading `/` or `!` in email.
pub const KNOWN_COMMANDS: &[&str] = &[
    "start",
    "help",
    "link",
    "todo",
    "todos",
    "remind",
    "reminder",
    "reminders",
    "expense",
    "finance",
    "bill",
    "bills",
    "portfolio",
    "journal",
    "life",
    "habit",
    "habits",
    "bookmark",
    "bookmarks",
    "footprint",
    "footprints",
    "ai",
    "ask",
    "ocr",
    "tts",
    "voice",
    "mcp",
    "webhook",
    "settings",
    "account",
];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DmarcError {
    #[error("no Authentication-Results header matching authserv-id `{0}` found")]
    AuthServNotFound(String),
    #[error("DMARC did not pass: {0}")]
    NotPassed(String),
    #[error("DMARC header.from `{dmarc_domain}` does not match sender domain `{sender_domain}`")]
    DomainMismatch {
        dmarc_domain: String,
        sender_domain: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InboundError {
    #[error("malformed MIME payload")]
    MalformedMime,
    #[error("missing From address")]
    MissingFrom,
    #[error("sender `{0}` is not in allowed senders list")]
    SenderNotAllowed(String),
    #[error("auto-reply dropped")]
    AutoReply,
    #[error("DMARC authentication failed: {0}")]
    Dmarc(#[from] DmarcError),
    #[error("missing Message-ID header")]
    MissingMessageId,
    #[error("empty email body and subject")]
    EmptyBody,
    #[error("invalid platform user id: {0}")]
    InvalidUserId(String),
}

/// Result of parsing an inbound email message.
pub struct ParsedEmail {
    pub message: IncomingMessage,
    pub text: InboundText,
    pub subject: Option<String>,
}

/// Verifies that DMARC passed in an `Authentication-Results` header issued by `authserv_id`.
pub fn verify_dmarc(
    headers: &[&str],
    authserv_id: &str,
    from_email: &str,
) -> Result<(), DmarcError> {
    let (_, sender_domain) = from_email
        .split_once('@')
        .ok_or_else(|| DmarcError::NotPassed("invalid from address".to_owned()))?;
    let sender_domain = sender_domain.trim().to_ascii_lowercase();

    for header in headers {
        let trimmed = header.trim();
        let authserv_token = trimmed
            .split([';', ' ', '\t', '\r', '\n', '/'])
            .next()
            .unwrap_or("")
            .trim();

        if !authserv_token.eq_ignore_ascii_case(authserv_id) {
            continue;
        }

        // Only evaluate the topmost Authentication-Results header for our MTA;
        // a lower (earlier/forged) header must never rescue a failed check.
        let lower = trimmed.to_ascii_lowercase();

        if !has_dmarc_pass(&lower) {
            return Err(DmarcError::NotPassed(trimmed.to_owned()));
        }

        if let Some(dmarc_domain) = extract_header_from_domain(&lower) {
            let aligned = sender_domain == dmarc_domain
                || sender_domain.ends_with(&format!(".{dmarc_domain}"));
            if !aligned {
                return Err(DmarcError::DomainMismatch {
                    dmarc_domain,
                    sender_domain,
                });
            }
        }

        return Ok(());
    }

    Err(DmarcError::AuthServNotFound(authserv_id.to_owned()))
}

fn has_dmarc_pass(header_lower: &str) -> bool {
    let mut rest = header_lower;
    while let Some(pos) = rest.find("dmarc") {
        let after = &rest[pos + 5..];
        let trimmed = after.trim_start();
        if let Some(after_eq) = trimmed.strip_prefix('=') {
            let result_token = after_eq
                .trim_start()
                .split([';', ' ', '\t', '\r', '\n', '(', ')'])
                .next()
                .unwrap_or("");
            if result_token == "pass" {
                return true;
            }
        }
        rest = after;
    }
    false
}

fn extract_header_from_domain(header_lower: &str) -> Option<String> {
    let pos = header_lower.find("header.from")?;
    let after = &header_lower[pos + 11..];
    let after_eq = after.trim_start().strip_prefix('=')?;
    let raw_val = after_eq
        .trim_start()
        .split([';', ' ', '\t', '\r', '\n', '(', ')'])
        .next()?;
    let domain = raw_val.trim_matches(['"', '\'']).trim();
    if domain.is_empty() {
        None
    } else {
        Some(domain.to_ascii_lowercase())
    }
}

/// Checks if an email is an automated reply or daemon bounce.
pub fn is_auto_reply(
    auto_submitted: Option<&str>,
    precedence: Option<&str>,
    from_addr: &str,
) -> bool {
    if let Some(val) = auto_submitted {
        let trimmed = val.trim().to_ascii_lowercase();
        if !trimmed.is_empty() && trimmed != "no" {
            return true;
        }
    }
    if let Some(val) = precedence {
        let lower = val.to_ascii_lowercase();
        if lower.contains("bulk") || lower.contains("list") || lower.contains("junk") {
            return true;
        }
    }
    let local = from_addr
        .split('@')
        .next()
        .unwrap_or(from_addr)
        .trim()
        .to_ascii_lowercase();
    if local.starts_with("mailer-daemon")
        || local.starts_with("postmaster")
        || local.starts_with("noreply")
        || local.starts_with("no-reply")
    {
        return true;
    }
    false
}

/// Strips email quotes (lines starting with `>`), attribution headers, and signatures.
pub fn strip_quotes_and_signatures(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // Signature delimiter (RFC 3676 "-- " or standalone "--")
        if *line == "-- " || trimmed == "--" {
            break;
        }

        // Standard forward/reply dividers
        if trimmed.starts_with("-----Original Message-----")
            || trimmed.starts_with("----- Original Message -----")
            || trimmed.starts_with("________________________________")
        {
            break;
        }

        // Attribution lines: "On ... wrote:"
        let is_attr = (trimmed.starts_with("On ") || trimmed.starts_with("on "))
            && (trimmed.ends_with(':') && trimmed.contains("wrote")
                || lines.get(idx + 1).is_some_and(|next| {
                    let next_trim = next.trim();
                    next_trim.ends_with(':') && next_trim.contains("wrote")
                }));
        if is_attr {
            break;
        }

        // Quoted line
        if trimmed.starts_with('>') {
            continue;
        }

        kept.push(*line);
    }

    kept.join("\n").trim().to_owned()
}

/// Parses email text into `InboundText::Command` or `InboundText::FreeText`.
pub fn parse_email_text(raw_text: &str) -> InboundText {
    let text = raw_text.trim();
    if let Some(rest) = text.strip_prefix('/').or_else(|| text.strip_prefix('!')) {
        let (head, args) = split_head(rest);
        let cmd = head.to_ascii_lowercase();
        if cmd.is_empty() {
            InboundText::FreeText(text.to_owned())
        } else {
            let full_body = format!("{cmd} {args}").trim().to_owned();
            InboundText::Command {
                command: cmd,
                raw_args: args.to_owned(),
                full_body,
            }
        }
    } else {
        let (head, rest) = split_head(text);
        let candidate = head.to_ascii_lowercase();
        if KNOWN_COMMANDS.contains(&candidate.as_str()) {
            let full_body = format!("{candidate} {rest}").trim().to_owned();
            InboundText::Command {
                command: candidate,
                raw_args: rest.to_owned(),
                full_body,
            }
        } else {
            InboundText::FreeText(text.to_owned())
        }
    }
}

fn split_head(s: &str) -> (&str, &str) {
    let trimmed = s.trim_start();
    match trimmed.split_once(char::is_whitespace) {
        Some((head, tail)) => (head, tail.trim_start()),
        None => (trimmed, ""),
    }
}

fn interaction_body(text: &InboundText) -> String {
    match text {
        InboundText::Command { full_body, .. } => full_body.clone(),
        InboundText::FreeText(t) => t.clone(),
    }
}

/// Parses an RFC 5322 MIME email into `ParsedEmail`.
pub fn parse_inbound_email(
    raw: &[u8],
    authserv_id: &str,
    allowed_senders: Option<&[String]>,
) -> Result<ParsedEmail, InboundError> {
    let parsed = MessageParser::default()
        .parse(raw)
        .ok_or(InboundError::MalformedMime)?;

    // Extract From address
    let (from_addr, display_name) = extract_from(&parsed)?;

    // Allowed senders check
    if let Some(allowed) = allowed_senders {
        if !allowed.is_empty() {
            let from_lower = from_addr.to_ascii_lowercase();
            let is_allowed = allowed.iter().any(|a| a.to_ascii_lowercase() == from_lower);
            if !is_allowed {
                return Err(InboundError::SenderNotAllowed(from_addr));
            }
        }
    }

    // Extract headers for auto-reply and DMARC
    let mut auto_submitted = None;
    let mut precedence = None;
    let mut auth_results = Vec::new();

    for (name, val) in parsed.headers_raw() {
        if name.eq_ignore_ascii_case("auto-submitted") {
            auto_submitted = Some(val);
        } else if name.eq_ignore_ascii_case("precedence")
            || name.eq_ignore_ascii_case("x-precedence")
        {
            precedence = Some(val);
        } else if name.eq_ignore_ascii_case("authentication-results") {
            auth_results.push(val);
        }
    }

    if is_auto_reply(auto_submitted, precedence, &from_addr) {
        return Err(InboundError::AutoReply);
    }

    // DMARC verification
    verify_dmarc(&auth_results, authserv_id, &from_addr)?;

    // Message-ID
    let msg_id = parsed
        .message_id()
        .ok_or(InboundError::MissingMessageId)?
        .trim()
        .trim_matches(['<', '>']);
    if msg_id.is_empty() {
        return Err(InboundError::MissingMessageId);
    }

    // Threading headers
    let in_reply_to = parsed
        .in_reply_to()
        .as_text()
        .map(|s| s.trim().trim_matches(['<', '>']).to_owned())
        .filter(|s| !s.is_empty());

    // Subject
    let subject = parsed.subject().map(|s| s.trim().to_owned());

    // Body extraction
    let body_text = parsed
        .body_text(0)
        .map(|s| s.into_owned())
        .or_else(|| parsed.body_html(0).map(|h| strip_html(&h)))
        .unwrap_or_default();

    let stripped_body = strip_quotes_and_signatures(&body_text);

    let final_text = if !stripped_body.is_empty() {
        stripped_body
    } else if let Some(subj) = &subject {
        subj.clone()
    } else {
        return Err(InboundError::EmptyBody);
    };

    let inbound_text = parse_email_text(&final_text);

    let user_id = PlatformUserId::parse(Channel::Email, &from_addr)
        .map_err(|e| InboundError::InvalidUserId(e.to_string()))?;
    let sender = PlatformIdentity::new(Channel::Email, user_id, display_name);

    let incoming = IncomingMessage {
        channel: Channel::Email,
        conversation: ConversationAddress::Email(from_addr),
        sender,
        message_id: MessageRef(msg_id.to_owned()),
        text: interaction_body(&inbound_text),
        reply_to: in_reply_to.map(MessageRef),
        quoted: None,
        is_direct: true,
        is_mentioned: true,
    };

    Ok(ParsedEmail {
        message: incoming,
        text: inbound_text,
        subject,
    })
}

fn extract_from(msg: &Message) -> Result<(String, Option<String>), InboundError> {
    let addr = msg
        .from()
        .and_then(|a| a.first())
        .ok_or(InboundError::MissingFrom)?;
    let email = addr.address().ok_or(InboundError::MissingFrom)?;
    let name = addr.name().map(str::to_string);
    Ok((email.trim().to_owned(), name))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS_FIXTURE: &str =
        include_str!("../../../../../contracts/channels/email_dmarc_pass.eml");
    const FAIL_FIXTURE: &str =
        include_str!("../../../../../contracts/channels/email_dmarc_fail.eml");
    const AUTO_FIXTURE: &str =
        include_str!("../../../../../contracts/channels/email_autoreply.eml");
    const QUOTE_FIXTURE: &str = include_str!("../../../../../contracts/channels/email_quoted.eml");

    #[test]
    fn test_verify_dmarc_pass() {
        let headers = ["mx.example.com; dkim=pass; dmarc=pass (p=REJECT) header.from=example.com"];
        assert!(verify_dmarc(&headers, "mx.example.com", "user@example.com").is_ok());
        // Subdomain alignment
        assert!(verify_dmarc(&headers, "mx.example.com", "user@sub.example.com").is_ok());
    }

    #[test]
    fn test_verify_dmarc_fail() {
        let headers = ["mx.example.com; dmarc=fail (p=REJECT) header.from=spoofed.com"];
        let res = verify_dmarc(&headers, "mx.example.com", "spoofer@spoofed.com");
        assert!(matches!(res, Err(DmarcError::NotPassed(_))));
    }

    #[test]
    fn test_verify_dmarc_domain_mismatch() {
        let headers = ["mx.example.com; dmarc=pass header.from=legit.com"];
        let res = verify_dmarc(&headers, "mx.example.com", "attacker@spoofed.com");
        assert!(matches!(
            res,
            Err(DmarcError::DomainMismatch {
                dmarc_domain,
                sender_domain
            }) if dmarc_domain == "legit.com" && sender_domain == "spoofed.com"
        ));
    }

    #[test]
    fn test_verify_dmarc_missing_authserv() {
        let headers = ["other-mx.example.com; dmarc=pass header.from=example.com"];
        let res = verify_dmarc(&headers, "mx.example.com", "user@example.com");
        assert!(matches!(res, Err(DmarcError::AuthServNotFound(_))));
    }

    #[test]
    fn test_is_auto_reply() {
        assert!(is_auto_reply(
            Some("auto-replied"),
            None,
            "user@example.com"
        ));
        assert!(is_auto_reply(
            Some("auto-generated"),
            None,
            "user@example.com"
        ));
        assert!(!is_auto_reply(Some("no"), None, "user@example.com"));
        assert!(is_auto_reply(None, Some("bulk"), "user@example.com"));
        assert!(is_auto_reply(None, Some("list"), "user@example.com"));
        assert!(is_auto_reply(None, None, "mailer-daemon@example.com"));
        assert!(is_auto_reply(None, None, "noreply@example.com"));
        assert!(is_auto_reply(None, None, "no-reply@example.com"));
        assert!(is_auto_reply(None, None, "postmaster@example.com"));
        assert!(!is_auto_reply(None, None, "alice@example.com"));
    }

    #[test]
    fn test_strip_quotes_and_signatures() {
        let text = "todo add Buy milk\n\nOn Fri, Oct 3, 2026 at 9:59 AM Bot wrote:\n> Some old text\n\n-- \nSignature";
        assert_eq!(strip_quotes_and_signatures(text), "todo add Buy milk");

        let text2 = "Here is my message\n-----Original Message-----\nFrom: bob";
        assert_eq!(strip_quotes_and_signatures(text2), "Here is my message");

        let text3 = "> quote\nreply text";
        assert_eq!(strip_quotes_and_signatures(text3), "reply text");
    }

    #[test]
    fn test_parse_email_text() {
        match parse_email_text("/todo add Milk") {
            InboundText::Command {
                command,
                raw_args,
                full_body,
            } => {
                assert_eq!(command, "todo");
                assert_eq!(raw_args, "add Milk");
                assert_eq!(full_body, "todo add Milk");
            }
            _ => panic!("expected command"),
        }

        match parse_email_text("!todo list") {
            InboundText::Command {
                command, raw_args, ..
            } => {
                assert_eq!(command, "todo");
                assert_eq!(raw_args, "list");
            }
            _ => panic!("expected command"),
        }

        match parse_email_text("todo list") {
            InboundText::Command {
                command, raw_args, ..
            } => {
                assert_eq!(command, "todo");
                assert_eq!(raw_args, "list");
            }
            _ => panic!("expected command"),
        }

        match parse_email_text("Can you help me with this calculation?") {
            InboundText::FreeText(t) => {
                assert_eq!(t, "Can you help me with this calculation?");
            }
            _ => panic!("expected free text"),
        }
    }

    #[test]
    fn test_parse_inbound_email_with_dmarc_pass_fixture() {
        let parsed = parse_inbound_email(PASS_FIXTURE.as_bytes(), "mx.example.com", None)
            .expect("should parse fixture");
        assert_eq!(parsed.message.channel, Channel::Email);
        assert_eq!(
            parsed.message.conversation,
            ConversationAddress::Email("alice@example.com".to_owned())
        );
        assert_eq!(
            parsed.message.sender.user_id().as_str(),
            "alice@example.com"
        );
        assert_eq!(parsed.message.sender.display_name(), Some("Alice User"));
        assert_eq!(parsed.message.message_id.0, "msg-pass-1234@example.com");
        assert!(parsed.message.is_direct);
        assert!(parsed.message.is_mentioned);

        match parsed.text {
            InboundText::FreeText(t) => {
                assert_eq!(t, "Please add this todo item.");
            }
            InboundText::Command { .. } => panic!("expected free text"),
        }
    }

    #[test]
    fn test_parse_inbound_email_with_dmarc_fail_fixture() {
        let res = parse_inbound_email(FAIL_FIXTURE.as_bytes(), "mx.example.com", None);
        assert!(matches!(res, Err(InboundError::Dmarc(_))));
    }

    #[test]
    fn test_parse_inbound_email_with_autoreply_fixture() {
        let res = parse_inbound_email(AUTO_FIXTURE.as_bytes(), "mx.example.com", None);
        assert!(matches!(res, Err(InboundError::AutoReply)));
    }

    #[test]
    fn test_parse_inbound_email_with_quoted_fixture() {
        let parsed = parse_inbound_email(QUOTE_FIXTURE.as_bytes(), "mx.example.com", None)
            .expect("should parse quote fixture");
        assert_eq!(parsed.message.message_id.0, "msg-quoted-1111@example.com");
        assert_eq!(
            parsed.message.reply_to,
            Some(MessageRef("bot-reply-0000@example.com".to_owned()))
        );
        match parsed.text {
            InboundText::Command {
                command, raw_args, ..
            } => {
                assert_eq!(command, "todo");
                assert_eq!(raw_args, "list");
            }
            _ => panic!("expected command"),
        }
    }

    #[test]
    fn test_allowed_senders_filtering() {
        let allowed = vec!["allowed@example.com".to_owned()];
        let res = parse_inbound_email(PASS_FIXTURE.as_bytes(), "mx.example.com", Some(&allowed));
        assert!(matches!(res, Err(InboundError::SenderNotAllowed(s)) if s == "alice@example.com"));

        let allowed_alice = vec!["alice@example.com".to_owned()];
        let res_ok = parse_inbound_email(
            PASS_FIXTURE.as_bytes(),
            "mx.example.com",
            Some(&allowed_alice),
        );
        assert!(res_ok.is_ok());
    }

    #[test]
    fn test_dmarc_rejects_forged_lower_authentication_results_header() {
        let headers = [
            "mx.example.com; dmarc=fail header.from=example.com",
            "mx.example.com; dmarc=pass header.from=example.com",
        ];
        let res = verify_dmarc(&headers, "mx.example.com", "alice@example.com");
        assert!(matches!(res, Err(DmarcError::NotPassed(_))));
    }
}
