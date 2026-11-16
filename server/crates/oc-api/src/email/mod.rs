//! The emails ObserveCraft sends: every one is `multipart/alternative` with a plain-text part and
//! an HTML part (`multipart/related`) whose images (the logo, a monitor's graph) are inline `cid:`
//! attachments. Images are never linked by URL: the install is often reachable only from inside a
//! network, where mail clients and image proxies can't fetch from it.
//!
//! - [`frame`]: the layout (status band), HTML and text; [`templates`]: each kind of email.
//! - [`chart`] and [`logo`]: the PNGs, drawn with tiny-skia.
//! - [`signed_link`]: the single-use links of On-Call's Acknowledge / Resolve buttons.

pub(crate) mod chart;
pub(crate) mod frame;
pub(crate) mod logo;
#[cfg(test)]
mod preview;
pub(crate) mod signed_link;
pub(crate) mod templates;

use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

/// An image of the HTML part, referenced as `cid:<cid>`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Image {
    pub cid: String,
    pub png: Vec<u8>,
}

/// A rendered email, ready to address.
#[derive(Clone, Debug, Default)]
pub(crate) struct Email {
    pub subject: String,
    pub text: String,
    pub html: String,
    pub images: Vec<Image>,
    /// `<…@…>`; when None the transport makes one up.
    pub message_id: Option<String>,
    /// The message this one answers (a recovery answers its alert), also its `References`.
    pub in_reply_to: Option<String>,
}

impl Email {
    /// Renders the frame; every email carries the logo.
    pub(crate) fn new(subject: impl Into<String>, frame: &frame::Frame) -> Email {
        let subject = subject.into();
        Email {
            html: frame.html(&subject),
            text: frame.text(),
            subject,
            images: vec![Image { cid: frame::LOGO_CID.into(), png: logo::png().to_vec() }],
            message_id: None,
            in_reply_to: None,
        }
    }

    /// The MIME message: alternative(text, related(html, images…)).
    pub(crate) fn message(&self, from: Mailbox, to: Mailbox) -> Result<Message, lettre::error::Error> {
        let mut b = Message::builder().from(from).to(to).subject(self.subject.clone());
        if let Some(id) = &self.message_id {
            b = b.message_id(Some(id.clone()));
        }
        if let Some(parent) = &self.in_reply_to {
            b = b.in_reply_to(parent.clone()).references(parent.clone());
        }
        let mut related = MultiPart::related().singlepart(SinglePart::html(self.html.clone()));
        for img in &self.images {
            related = related.singlepart(Attachment::new_inline(img.cid.clone()).body(img.png.clone(), ContentType::parse("image/png").expect("image/png")));
        }
        b.multipart(MultiPart::alternative().singlepart(SinglePart::plain(self.text.clone())).multipart(related))
    }
}

/// The domain of the sender address (`alerts@observecraft.example.com` → `observecraft.example.com`),
/// for Message-IDs.
pub(crate) fn id_domain(from: &str) -> String {
    let addr = from.rsplit_once('<').map(|(_, a)| a.trim_end_matches('>')).unwrap_or(from);
    addr.rsplit_once('@').map(|(_, d)| d.trim().to_ascii_lowercase()).filter(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')).unwrap_or_else(|| "observecraft.local".into())
}

/// Why an email wasn't sent: nothing to retry (configuration, address) or a delivery failure.
pub(crate) enum SendError {
    Invalid(String),
    Failed(String),
}

/// Sends through the install's SMTP server (`OC_SMTP_URL`).
pub(crate) async fn send(smtp_url: &str, from: &str, to: &str, email: &Email) -> Result<(), SendError> {
    let (Ok(from), Ok(to_addr)) = (from.parse::<Mailbox>(), to.parse::<Mailbox>()) else {
        return Err(SendError::Invalid(format!("Invalid email address {to}")));
    };
    let msg = email.message(from, to_addr).map_err(|e| SendError::Failed(e.to_string()))?;
    let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(smtp_url).map_err(|e| SendError::Invalid(format!("Invalid OC_SMTP_URL: {e}")))?.build();
    transport.send(msg).await.map(|_| ()).map_err(|e| SendError::Failed(format!("SMTP: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use frame::{Button, ButtonStyle, Footer, Frame, Picture, Product};

    fn sample() -> Email {
        let f = Frame {
            product: Product::Core,
            band_color: frame::colors::ALERT,
            band: "Triggered".into(),
            title: "High error rate".into(),
            lead: vec![],
            message: None,
            picture: Some(Picture { cid: "graph@observecraft".into(), alt: "graph".into(), width: 516, height: 120 }),
            rows: vec![],
            buttons: vec![Button::new("View monitor", "https://oc.example.com/monitors/1", ButtonStyle::Primary)],
            note: None,
            footer: Footer { why: "Why.".into(), link_label: "Settings".into(), url: "https://oc.example.com/account/profile".into() },
            host: "oc.example.com".into(),
        };
        let mut e = Email::new("[Triggered] High error rate", &f);
        e.images.push(Image { cid: "graph@observecraft".into(), png: chart::render(&chart::Chart { points: vec![(0, 1.0), (60_000, 2.0)], from: 0, to: 120_000, thresholds: vec![], color: frame::colors::ALERT, event_at: None }).unwrap() });
        e
    }

    fn formatted(e: &Email) -> String {
        let m = e.message("ObserveCraft <alerts@oc.example.com>".parse().unwrap(), "ana@example.com".parse().unwrap()).unwrap();
        String::from_utf8_lossy(&m.formatted()).into_owned()
    }

    #[test]
    fn mime_is_alternative_text_then_related_html_with_its_images() {
        let raw = formatted(&sample());
        let alt = raw.find("multipart/alternative").expect("alternative");
        let plain = raw.find("text/plain").expect("plain");
        let related = raw.find("multipart/related").expect("related");
        let html = raw.find("text/html").expect("html");
        assert!(alt < plain && plain < related && related < html, "{raw}");
        // Every cid the HTML references is an inline part of the related part.
        let e = sample();
        let mut cids = Vec::new();
        let mut rest = e.html.as_str();
        while let Some(i) = rest.find("cid:") {
            let tail = &rest[i + 4..];
            cids.push(tail[..tail.find('"').unwrap()].to_string());
            rest = tail;
        }
        assert_eq!(cids, vec!["logo@observecraft", "graph@observecraft"]);
        for cid in &cids {
            assert!(raw.contains(&format!("Content-ID: <{cid}>")), "{cid} missing");
            assert!(e.images.iter().any(|i| &i.cid == cid));
        }
        assert_eq!(raw.matches("Content-Type: image/png").count(), 2);
        assert!(raw.contains("Content-Disposition: inline"));
    }

    #[test]
    fn threading_headers() {
        let mut e = sample();
        e.message_id = Some("<monitor-1.event-9@oc.example.com>".into());
        e.in_reply_to = Some("<monitor-1.event-7@oc.example.com>".into());
        let raw = formatted(&e);
        assert!(raw.contains("Message-ID: <monitor-1.event-9@oc.example.com>"), "{raw}");
        assert!(raw.contains("In-Reply-To: <monitor-1.event-7@oc.example.com>"));
        assert!(raw.contains("References: <monitor-1.event-7@oc.example.com>"));
    }

    #[test]
    fn message_id_domain_comes_from_the_sender() {
        assert_eq!(id_domain("ObserveCraft <alerts@OC.Example.com>"), "oc.example.com");
        assert_eq!(id_domain("alerts@oc.example.com"), "oc.example.com");
        assert_eq!(id_domain("nobody"), "observecraft.local");
    }
}
