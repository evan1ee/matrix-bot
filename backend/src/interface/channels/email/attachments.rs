//! Receipt attachments of an inbound email. Pure: bytes in, verdicts out.
//!
//! The declared MIME type is the sender's claim, so the bytes decide what a
//! part is. Limits bound what one message can cost: a 30 MiB budget, 10 MiB a
//! part and five receipts, so a hostile message cannot fan out into an OCR job
//! per megabyte.

use mail_parser::{Message, MimeHeaders};

use crate::domain::finance::receipt::sniff_receipt_type;

pub const MAX_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_MESSAGE_BYTES: usize = 30 * 1024 * 1024;
pub const MAX_RECEIPTS_PER_MESSAGE: usize = 5;

/// An image or PDF part accepted for OCR (the inference service rasterises
/// the first pages of a PDF).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundAttachment {
    pub filename: String,
    pub content_type: &'static str,
    pub extension: &'static str,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    TooLarge,
    MessageTooLarge,
    TooMany,
    NotAReceipt,
}

impl SkipReason {
    pub fn describe(self) -> &'static str {
        match self {
            Self::TooLarge => "larger than the 10 MiB limit",
            Self::MessageTooLarge => "this message already exceeded the 30 MiB limit",
            Self::TooMany => "only the first 5 receipts of a message are processed",
            Self::NotAReceipt => "not a JPEG, PNG, WebP or PDF file",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedAttachment {
    pub filename: String,
    pub reason: SkipReason,
}

#[derive(Debug, Default)]
pub struct Extracted {
    pub accepted: Vec<InboundAttachment>,
    pub skipped: Vec<SkippedAttachment>,
}

/// Sorts a message's attachment parts into receipts to process and parts to
/// report back. Parts with a `Content-ID` are images embedded in the HTML body
/// (logos, tracking pixels), not receipts, and are ignored without a report.
pub fn extract_attachments(msg: &Message) -> Extracted {
    let mut out = Extracted::default();
    let mut budget = 0usize;

    for (idx, part) in msg.attachments().enumerate() {
        if part.content_id().is_some() {
            continue;
        }
        let filename = part
            .attachment_name()
            .map_or_else(|| format!("attachment-{}", idx + 1), str::to_owned);
        let data = part.contents();

        let Some((extension, content_type)) = sniff_receipt_type(data) else {
            out.skipped.push(skip(filename, SkipReason::NotAReceipt));
            continue;
        };
        let verdict = if data.len() > MAX_ATTACHMENT_BYTES {
            Some(SkipReason::TooLarge)
        } else if budget + data.len() > MAX_MESSAGE_BYTES {
            Some(SkipReason::MessageTooLarge)
        } else if out.accepted.len() >= MAX_RECEIPTS_PER_MESSAGE {
            Some(SkipReason::TooMany)
        } else {
            None
        };
        match verdict {
            Some(reason) => out.skipped.push(skip(filename, reason)),
            None => {
                budget += data.len();
                out.accepted.push(InboundAttachment {
                    filename,
                    content_type,
                    extension,
                    data: data.to_vec(),
                });
            }
        }
    }
    out
}

fn skip(filename: String, reason: SkipReason) -> SkippedAttachment {
    SkippedAttachment { filename, reason }
}

#[cfg(test)]
mod tests {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use mail_parser::MessageParser;

    use super::*;

    const PNG_HEAD: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    fn png(len: usize) -> Vec<u8> {
        let mut bytes = PNG_HEAD.to_vec();
        bytes.resize(len.max(PNG_HEAD.len()), 0);
        bytes
    }

    /// One MIME part: `(declared type, filename, extra header, bytes)`.
    type Part<'a> = (&'a str, &'a str, &'a str, Vec<u8>);

    fn mime(parts: &[Part]) -> Vec<u8> {
        let mut raw = String::from(
            "From: alice@example.com\r\nSubject: Receipt\r\nMessage-ID: <m1@example.com>\r\n\
             Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
             --b\r\nContent-Type: text/plain\r\n\r\nlunch\r\n",
        );
        for (ctype, name, extra, data) in parts {
            raw.push_str(&format!(
                "--b\r\nContent-Type: {ctype}; name=\"{name}\"\r\n\
                 Content-Disposition: attachment; filename=\"{name}\"\r\n{extra}\
                 Content-Transfer-Encoding: base64\r\n\r\n{}\r\n",
                STANDARD.encode(data)
            ));
        }
        raw.push_str("--b--\r\n");
        raw.into_bytes()
    }

    fn extract(raw: &[u8]) -> Extracted {
        extract_attachments(&MessageParser::default().parse(raw).unwrap())
    }

    #[test]
    fn a_png_part_is_accepted_with_the_sniffed_type() {
        let got = extract(&mime(&[("image/png", "r.png", "", png(64))]));
        assert_eq!(got.accepted.len(), 1);
        assert_eq!(got.accepted[0].filename, "r.png");
        assert_eq!(got.accepted[0].content_type, "image/png");
        assert_eq!(got.accepted[0].extension, "png");
        assert_eq!(got.accepted[0].data, png(64));
        assert!(got.skipped.is_empty());
    }

    #[test]
    fn a_pdf_is_accepted_with_the_sniffed_type() {
        let got = extract(&mime(&[(
            "application/pdf",
            "inv.pdf",
            "",
            b"%PDF-1.7
"
            .to_vec(),
        )]));
        assert!(got.skipped.is_empty());
        assert_eq!(got.accepted.len(), 1);
        assert_eq!(got.accepted[0].content_type, "application/pdf");
        assert_eq!(got.accepted[0].extension, "pdf");
    }

    #[test]
    fn an_oversized_pdf_is_skipped_like_an_image() {
        let mut big = b"%PDF-1.7
"
        .to_vec();
        big.resize(MAX_ATTACHMENT_BYTES + 1, 0);
        let got = extract(&mime(&[("application/pdf", "big.pdf", "", big)]));
        assert_eq!(got.skipped[0].reason, SkipReason::TooLarge);
    }

    #[test]
    fn the_declared_type_is_not_trusted() {
        let got = extract(&mime(&[(
            "image/png",
            "fake.png",
            "",
            b"just text".to_vec(),
        )]));
        assert!(got.accepted.is_empty());
        assert_eq!(got.skipped[0].reason, SkipReason::NotAReceipt);
    }

    #[test]
    fn an_embedded_logo_is_ignored_without_a_report() {
        let got = extract(&mime(&[(
            "image/png",
            "logo.png",
            "Content-ID: <logo@example.com>\r\n",
            png(64),
        )]));
        assert!(got.accepted.is_empty());
        assert!(got.skipped.is_empty());
    }

    #[test]
    fn an_oversized_part_is_skipped() {
        let got = extract(&mime(&[(
            "image/png",
            "big.png",
            "",
            png(MAX_ATTACHMENT_BYTES + 1),
        )]));
        assert_eq!(got.skipped[0].reason, SkipReason::TooLarge);
    }

    #[test]
    fn parts_beyond_the_message_budget_are_skipped() {
        let nine_mib = 9 * 1024 * 1024;
        let parts: Vec<Part> = (0..4)
            .map(|i| {
                (
                    "image/png",
                    ["a.png", "b.png", "c.png", "d.png"][i],
                    "",
                    png(nine_mib),
                )
            })
            .collect();
        let got = extract(&mime(&parts));
        assert_eq!(got.accepted.len(), 3);
        assert_eq!(got.skipped[0].reason, SkipReason::MessageTooLarge);
    }

    #[test]
    fn receipts_beyond_the_count_cap_are_skipped() {
        let parts: Vec<Part> = (0..6)
            .map(|_| ("image/png", "r.png", "", png(64)))
            .collect();
        let got = extract(&mime(&parts));
        assert_eq!(got.accepted.len(), MAX_RECEIPTS_PER_MESSAGE);
        assert_eq!(got.skipped[0].reason, SkipReason::TooMany);
    }
}
