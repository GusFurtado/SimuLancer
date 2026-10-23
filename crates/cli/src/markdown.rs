//! A project's documentation: Markdown rendered to HTML on the server, so
//! every client shows the same thing and the pages load no Markdown library.
//!
//! The documentation is shared with everyone who opens the project, so the
//! output is safe to put in a page: HTML written in the Markdown is shown as
//! text, links go only to web pages, e-mail addresses or within the page,
//! and images become links to them (the pages load nothing they don't ship).

use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd, html};

pub fn to_html(text: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    // Links can't nest, so one flag per kind says which end to drop or rewrite.
    let mut dropped_link = false;
    let mut image_as_link = false;
    let mut in_link = false;
    let events = Parser::new_ext(text, options).filter_map(|event| match event {
        Event::Html(t) | Event::InlineHtml(t) => Some(Event::Text(t)),
        Event::Start(Tag::HtmlBlock) => Some(Event::Start(Tag::Paragraph)),
        Event::End(TagEnd::HtmlBlock) => Some(Event::End(TagEnd::Paragraph)),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            if safe_url(&dest_url) {
                in_link = true;
                Some(Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    title,
                    id,
                }))
            } else {
                dropped_link = true;
                None
            }
        }
        Event::End(TagEnd::Link) => {
            in_link = false;
            if std::mem::take(&mut dropped_link) {
                None
            } else {
                Some(Event::End(TagEnd::Link))
            }
        }
        // An image is shown as a link with its description; inside a link,
        // or with an unsafe address, as the description alone.
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            if in_link || dropped_link || !safe_url(&dest_url) {
                None
            } else {
                image_as_link = true;
                Some(Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    title,
                    id,
                }))
            }
        }
        Event::End(TagEnd::Image) => {
            std::mem::take(&mut image_as_link).then_some(Event::End(TagEnd::Link))
        }
        other => Some(other),
    });
    let mut out = String::new();
    html::push_html(&mut out, events);
    out
}

/// Web pages, e-mail, and addresses without a scheme (`#section`, `notes.md`).
fn safe_url(url: &CowStr) -> bool {
    let url: String = url
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect();
    match url.find(':') {
        Some(colon) if !url[..colon].contains(['/', '?', '#']) => {
            let scheme = url[..colon].to_ascii_lowercase();
            matches!(scheme.as_str(), "http" | "https" | "mailto")
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::to_html;

    #[test]
    fn renders_headings_lists_tables_and_links() {
        let html = to_html(
            "# Assumptions\n\n- Failures are **Weibull**\n- [Data](https://example.com/data)\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
        );
        assert!(html.contains("<h1>Assumptions</h1>"), "{html}");
        assert!(
            html.contains("<li>Failures are <strong>Weibull</strong></li>"),
            "{html}"
        );
        assert!(
            html.contains(r#"<a href="https://example.com/data">Data</a>"#),
            "{html}"
        );
        assert!(html.contains("<table>"), "{html}");
    }

    #[test]
    fn html_in_the_markdown_is_text() {
        let html = to_html("<script>alert(1)</script>\n\nA <b onclick=\"x()\">bold</b> claim\n");
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("<b "), "{html}");
        assert!(
            html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "{html}"
        );
        assert!(
            html.contains("&lt;b onclick=\"x()\"&gt;bold&lt;/b&gt;"),
            "{html}"
        );
    }

    #[test]
    fn links_only_to_web_pages_mail_or_within() {
        let html = to_html(
            "[a](javascript:alert(1)) [b]( JavaScript:x) [c](data:text/html,x) [d](mailto:x@y.z) [e](#notes) [f](HTTPS://x.org)",
        );
        assert!(!html.to_lowercase().contains("javascript"), "{html}");
        assert!(!html.contains("data:"), "{html}");
        assert!(html.contains("<p>a b c "), "{html}");
        assert!(html.contains(r#"href="mailto:x@y.z""#), "{html}");
        assert!(html.contains(r##"href="#notes""##), "{html}");
        assert!(html.contains(r#"href="HTTPS://x.org""#), "{html}");
    }

    #[test]
    fn images_become_links() {
        let html = to_html(
            "![The line](https://x.org/line.png) ![x](javascript:y) [![in](https://x.org/i.png)](https://x.org)",
        );
        assert!(!html.contains("<img"), "{html}");
        assert!(
            html.contains(r#"<a href="https://x.org/line.png">The line</a>"#),
            "{html}"
        );
        assert!(!html.contains("javascript"), "{html}");
        assert!(html.contains(r#"<a href="https://x.org">in</a>"#), "{html}");
    }
}
