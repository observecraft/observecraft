//! The one layout every email uses (design "A · Status band"): a white card with the logo, a
//! full-width band in the state's colour, the title, one sentence, the graph, a key/value table,
//! buttons and a footer saying why the email was sent. Written for mail clients: tables and inline
//! styles only, no external fonts or SVG, images by `cid:`. The same content renders as plain text.
//!
//! Every dynamic value goes through [`escape`]: monitor names, messages and tags are user input.

use std::fmt::Write;

/// Escapes text for HTML element content and quoted attribute values.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Only http(s) links become buttons: a `javascript:` URL in an email would be a trap.
fn safe_url(url: &str) -> Option<String> {
    let lower = url.trim().to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")).then(|| escape(url.trim()))
}

pub(crate) const FONT: &str = "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif";
const MONO: &str = "ui-monospace,SFMono-Regular,Menlo,Consolas,monospace";
pub(crate) const INK: &str = "#1c2b29";
const MUTED: &str = "#61716e";
const LINE: &str = "#edf1f0";
const LINK: &str = "#0b6e69";
/// Content width inside the card (560 minus 2 × 22 of padding).
pub(crate) const CONTENT_WIDTH: u32 = 516;

/// The state colours of the band (and of the graph).
pub(crate) mod colors {
    pub const ALERT: &str = "#b42318";
    pub const WARN: &str = "#9a5b00";
    pub const NO_DATA: &str = "#5b6b68";
    pub const OK: &str = "#12805c";
    pub const ON_CALL: &str = "#7a1a12";
    pub const BRAND: &str = "#0b5f5c";
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Product {
    Core,
    OnCall,
}

impl Product {
    fn name(self) -> &'static str {
        match self {
            Product::Core => "ObserveCraft",
            Product::OnCall => "ObserveCraft On-Call",
        }
    }
}

/// Text with bold parts (the value in "Error rate is **7.1%**").
#[derive(Clone, Debug)]
pub(crate) enum Run {
    Plain(String),
    Strong(String),
}

#[derive(Clone, Debug)]
pub(crate) enum Cell {
    Text(String),
    /// Shown as code chips (group tags, a query).
    Codes(Vec<String>),
}

#[derive(Clone, Debug)]
pub(crate) enum Row {
    Pair(String, Cell),
    /// A sub-heading inside the table (a report's widget).
    Heading(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ButtonStyle {
    Primary,
    /// Red: On-Call's Acknowledge.
    Danger,
    Ghost,
}

#[derive(Clone, Debug)]
pub(crate) struct Button {
    pub label: String,
    pub url: String,
    pub style: ButtonStyle,
}

impl Button {
    pub(crate) fn new(label: &str, url: impl Into<String>, style: ButtonStyle) -> Self {
        Button { label: label.into(), url: url.into(), style }
    }
}

/// A picture in the email, referenced as `cid:<cid>`.
#[derive(Clone, Debug)]
pub(crate) struct Picture {
    pub cid: String,
    pub alt: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct Footer {
    /// "You get this because the monitor mentions you."
    pub why: String,
    pub link_label: String,
    pub url: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Frame {
    pub product: Product,
    pub band_color: &'static str,
    /// "Triggered · P2"; shown in capitals.
    pub band: String,
    pub title: String,
    pub lead: Vec<Run>,
    /// The sender's own text (a monitor message, a report note), kept as written.
    pub message: Option<String>,
    pub picture: Option<Picture>,
    pub rows: Vec<Row>,
    pub buttons: Vec<Button>,
    /// Small print under the buttons.
    pub note: Option<String>,
    pub footer: Footer,
    /// The install's host, under the footer.
    pub host: String,
}

/// The `cid` of the logo every email carries.
pub(crate) const LOGO_CID: &str = "logo@observecraft";

fn button_html(b: &Button) -> Option<String> {
    let url = safe_url(&b.url)?;
    let (bg, border, color) = match b.style {
        ButtonStyle::Primary => (LINK, LINK, "#ffffff"),
        ButtonStyle::Danger => (colors::ALERT, colors::ALERT, "#ffffff"),
        ButtonStyle::Ghost => ("#ffffff", "#c9d6d3", INK),
    };
    // Outlook ignores padding on links: the hidden <i> spacers give it the button's size.
    Some(format!(
        "<a href=\"{url}\" target=\"_blank\" style=\"display:inline-block;margin:0 8px 8px 0;padding:10px 18px;background:{bg};border:1px solid {border};border-radius:6px;\
         font-family:{FONT};font-size:14px;line-height:20px;font-weight:600;color:{color};text-decoration:none;mso-padding-alt:0;\">\
         <!--[if mso]><i style=\"mso-font-width:150%;mso-text-raise:20pt\" hidden>&#8202;&#8202;</i><![endif]-->\
         <span style=\"mso-text-raise:10pt;color:{color};\">{}</span>\
         <!--[if mso]><i style=\"mso-font-width:150%\" hidden>&#8202;&#8202;&#8203;</i><![endif]--></a>",
        escape(&b.label)
    ))
}

fn runs_html(runs: &[Run], strong_color: &str) -> String {
    runs.iter()
        .map(|r| match r {
            Run::Plain(t) => escape(t),
            Run::Strong(t) => format!("<b style=\"color:{strong_color};\">{}</b>", escape(t)),
        })
        .collect()
}

fn runs_text(runs: &[Run]) -> String {
    runs.iter().map(|r| match r { Run::Plain(t) | Run::Strong(t) => t.as_str() }).collect()
}

fn code_html(s: &str) -> String {
    format!("<code style=\"font-family:{MONO};font-size:12px;background:#f1f5f4;color:{INK};padding:1px 5px;border-radius:4px;word-break:break-word;overflow-wrap:anywhere;\">{}</code>", escape(s))
}

fn cell_html(c: &Cell) -> String {
    match c {
        Cell::Text(t) => escape(t),
        Cell::Codes(cs) => cs.iter().map(|s| code_html(s)).collect::<Vec<_>>().join(" "),
    }
}

fn cell_text(c: &Cell) -> String {
    match c {
        Cell::Text(t) => t.clone(),
        Cell::Codes(cs) => cs.join(" "),
    }
}

impl Frame {
    /// The HTML part. `subject` is the document title (some clients show it).
    pub(crate) fn html(&self, subject: &str) -> String {
        let mut h = String::with_capacity(8 * 1024);
        let td = format!("font-family:{FONT};font-size:14px;line-height:1.5;color:{INK};");
        let _ = write!(
            h,
            "<!DOCTYPE html><html lang=\"en\" xmlns=\"http://www.w3.org/1999/xhtml\"><head><meta charset=\"utf-8\">\
             <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"X-UA-Compatible\" content=\"IE=edge\">\
             <meta name=\"color-scheme\" content=\"light\"><meta name=\"supported-color-schemes\" content=\"light\"><title>{}</title>\
             <style>:root{{color-scheme:light;supported-color-schemes:light;}}\
             @media only screen and (max-width:620px){{.oc-card{{width:100% !important;}}.oc-pad{{padding-left:16px !important;padding-right:16px !important;}}.oc-outer{{padding:12px 6px !important;}}}}</style></head>\
             <body style=\"margin:0;padding:0;background:#eef2f1;-webkit-text-size-adjust:100%;\">",
            escape(subject)
        );
        // The preview line inbox lists show next to the subject.
        let _ = write!(h, "<div style=\"display:none;max-height:0;overflow:hidden;mso-hide:all;\">{}</div>", escape(&runs_text(&self.lead)));
        let _ = write!(
            h,
            "<table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"background:#eef2f1;\"><tr><td align=\"center\" class=\"oc-outer\" style=\"padding:24px 12px;\">\
             <!--[if mso]><table role=\"presentation\" width=\"560\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\"><tr><td><![endif]-->\
             <table role=\"presentation\" class=\"oc-card\" width=\"560\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"width:560px;max-width:560px;background:#ffffff;border:1px solid #dfe7e5;border-radius:10px;border-collapse:separate;\">"
        );
        // Header: logo and product.
        let _ = write!(
            h,
            "<tr><td class=\"oc-pad\" style=\"padding:16px 22px;border-bottom:1px solid {LINE};\"><table role=\"presentation\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\"><tr>\
             <td style=\"padding-right:8px;vertical-align:middle;\"><img src=\"cid:{LOGO_CID}\" width=\"24\" height=\"24\" alt=\"\" style=\"display:block;border:0;outline:none;\"></td>\
             <td style=\"font-family:{FONT};font-size:15px;line-height:24px;font-weight:700;color:#073f3d;vertical-align:middle;\">{}</td></tr></table></td></tr>",
            self.product.name()
        );
        // The state band.
        let _ = write!(
            h,
            "<tr><td class=\"oc-pad\" bgcolor=\"{c}\" style=\"background:{c};padding:10px 22px;font-family:{FONT};font-size:13px;line-height:18px;font-weight:700;letter-spacing:.04em;text-transform:uppercase;color:#ffffff;\">&#9679;&nbsp;{}</td></tr>",
            escape(&self.band),
            c = self.band_color
        );
        // Body.
        let _ = write!(h, "<tr><td class=\"oc-pad\" style=\"padding:20px 22px 14px;{td}\">");
        let _ = write!(h, "<div style=\"font-family:{FONT};font-size:18px;line-height:1.3;font-weight:700;color:{INK};\">{}</div>", escape(&self.title));
        if !self.lead.is_empty() {
            let _ = write!(h, "<p style=\"margin:12px 0 0;font-family:{FONT};font-size:14px;line-height:1.5;color:{MUTED};\">{}</p>", runs_html(&self.lead, self.band_color));
        }
        if let Some(m) = self.message.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
            let _ = write!(
                h,
                "<div style=\"margin:14px 0 0;padding:10px 12px;background:#f6f9f8;border-left:3px solid #c9d6d3;border-radius:4px;font-family:{FONT};font-size:13px;line-height:1.5;color:{INK};white-space:pre-wrap;word-break:break-word;\">{}</div>",
                escape(m)
            );
        }
        if let Some(p) = &self.picture {
            let _ = write!(
                h,
                "<img src=\"cid:{}\" width=\"{}\" height=\"{}\" alt=\"{}\" style=\"display:block;width:100%;max-width:{}px;height:auto;margin:16px 0 0;border:1px solid {LINE};border-radius:6px;\">",
                escape(&p.cid),
                p.width,
                p.height,
                escape(&p.alt),
                p.width
            );
        }
        if !self.rows.is_empty() {
            let _ = write!(h, "<table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"margin:16px 0 0;border-collapse:collapse;\">");
            for r in &self.rows {
                match r {
                    Row::Pair(k, v) => {
                        let _ = write!(
                            h,
                            "<tr><td width=\"34%\" style=\"width:34%;padding:6px 12px 6px 0;border-top:1px solid {LINE};vertical-align:top;font-family:{FONT};font-size:13px;line-height:1.45;color:{MUTED};\">{}</td>\
                             <td style=\"padding:6px 0;border-top:1px solid {LINE};vertical-align:top;font-family:{FONT};font-size:13px;line-height:1.45;color:{INK};word-break:break-word;\">{}</td></tr>",
                            escape(k),
                            cell_html(v)
                        );
                    }
                    Row::Heading(t) => {
                        let _ = write!(
                            h,
                            "<tr><td colspan=\"2\" style=\"padding:12px 0 4px;border-top:1px solid {LINE};font-family:{FONT};font-size:13px;line-height:1.4;font-weight:700;color:{INK};\">{}</td></tr>",
                            escape(t)
                        );
                    }
                }
            }
            h.push_str("</table>");
        }
        let buttons: Vec<String> = self.buttons.iter().filter_map(button_html).collect();
        if !buttons.is_empty() {
            let _ = write!(h, "<div style=\"margin:18px 0 0;\">{}</div>", buttons.join(""));
        }
        if let Some(n) = &self.note {
            let _ = write!(h, "<p style=\"margin:6px 0 0;font-family:{FONT};font-size:12px;line-height:1.5;color:{MUTED};\">{}</p>", escape(n));
        }
        h.push_str("</td></tr></table>");
        // Footer, outside the card.
        let link = safe_url(&self.footer.url).map(|u| format!(" <a href=\"{u}\" target=\"_blank\" style=\"color:{LINK};text-decoration:underline;\">{}</a>", escape(&self.footer.link_label))).unwrap_or_default();
        let _ = write!(
            h,
            "<table role=\"presentation\" class=\"oc-card\" width=\"560\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"width:560px;max-width:560px;\"><tr>\
             <td align=\"center\" style=\"padding:16px 4px 0;font-family:{FONT};font-size:12px;line-height:1.5;color:#7a8a87;\">{}{link}<br>ObserveCraft &middot; {}</td></tr></table>",
            escape(&self.footer.why),
            escape(&self.host)
        );
        h.push_str("<!--[if mso]></td></tr></table><![endif]--></td></tr></table></body></html>");
        h
    }

    /// The plain-text part: the same content, links written out.
    pub(crate) fn text(&self) -> String {
        let mut t = String::new();
        let _ = writeln!(t, "{} | {}", self.product.name(), self.band.to_uppercase());
        let _ = writeln!(t, "\n{}", self.title);
        if !self.lead.is_empty() {
            let _ = writeln!(t, "\n{}", runs_text(&self.lead));
        }
        if let Some(m) = self.message.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
            let _ = writeln!(t, "\n{m}");
        }
        if !self.rows.is_empty() {
            t.push('\n');
            for r in &self.rows {
                match r {
                    Row::Pair(k, v) => {
                        let _ = writeln!(t, "{k}: {}", cell_text(v));
                    }
                    Row::Heading(h) => {
                        let _ = writeln!(t, "\n{h}");
                    }
                }
            }
        }
        if !self.buttons.is_empty() {
            t.push('\n');
            for b in &self.buttons {
                let _ = writeln!(t, "{}: {}", b.label, b.url);
            }
        }
        if let Some(n) = &self.note {
            let _ = writeln!(t, "\n{n}");
        }
        let _ = write!(t, "\n-- \n{} {}: {}\n", self.footer.why, self.footer.link_label, self.footer.url);
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(title: &str, message: &str) -> Frame {
        Frame {
            product: Product::Core,
            band_color: colors::ALERT,
            band: "Triggered · P2".into(),
            title: title.into(),
            lead: vec![Run::Plain("Value is ".into()), Run::Strong("7.1".into()), Run::Plain(", above 5.".into())],
            message: Some(message.into()),
            picture: None,
            rows: vec![Row::Pair("Group".into(), Cell::Codes(vec!["service:<order-service>".into()]))],
            buttons: vec![Button::new("View monitor", "https://oc.example.com/monitors/1", ButtonStyle::Primary), Button::new("Evil", "javascript:alert(1)", ButtonStyle::Ghost)],
            note: None,
            footer: Footer { why: "You get this because the monitor mentions you.".into(), link_label: "Notification settings".into(), url: "https://oc.example.com/account/profile?section=email".into() },
            host: "oc.example.com".into(),
        }
    }

    #[test]
    fn escapes_every_dynamic_value() {
        assert_eq!(escape("<b a=\"x\">&'"), "&lt;b a=&quot;x&quot;&gt;&amp;&#39;");
        let f = frame("<script>alert(1)</script> & co", "Call <img src=x onerror=alert(1)> \"now\"");
        let html = f.html("[Triggered] <script>");
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img src=x"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt; &amp; co"));
        assert!(html.contains("Call &lt;img src=x onerror=alert(1)&gt; &quot;now&quot;"));
        assert!(html.contains("service:&lt;order-service&gt;"));
        // Only http(s) links become buttons.
        assert!(!html.contains("javascript:"));
        assert!(html.contains("href=\"https://oc.example.com/monitors/1\""));
    }

    #[test]
    fn text_part_has_the_content_and_links() {
        let t = frame("High error rate", "Check the runbook").text();
        assert!(t.starts_with("ObserveCraft | TRIGGERED · P2\n\nHigh error rate\n\nValue is 7.1, above 5.\n"));
        assert!(t.contains("Group: service:<order-service>\n"));
        assert!(t.contains("View monitor: https://oc.example.com/monitors/1\n"));
        assert!(t.ends_with("You get this because the monitor mentions you. Notification settings: https://oc.example.com/account/profile?section=email\n"));
    }

    #[test]
    fn html_is_table_based_with_the_logo_by_cid() {
        let html = frame("t", "m").html("s");
        assert!(html.contains(&format!("src=\"cid:{LOGO_CID}\"")));
        assert!(html.contains("bgcolor=\"#b42318\""));
        assert!(!html.contains("<svg"));
        assert!(!html.contains("src=\"http"));
    }
}
