//! Readable notices from Canvas HTML, excluding hidden browser controls.
use html5ever::{
    parse_document,
    serialize::{SerializeOpts, serialize},
    tendril::TendrilSink,
};
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanvasFileLink {
    pub course: String,
    pub file_id: String,
    pub title: String,
}

/// A Canvas HTML body with its file actions and plain text extracted once.
/// Preparing this on every frame parses the same document repeatedly; screens
/// cache it by page identity instead.
#[derive(Clone, Debug)]
pub struct PreparedCanvasBody {
    pub body: gpui::SharedString,
    pub links: Vec<CanvasFileLink>,
    pub text: String,
}

impl PreparedCanvasBody {
    /// A body that only repeats the name of its single file link would show
    /// the file twice, so the caller skips the text.
    pub fn only_link_body(&self) -> bool {
        self.links.len() == 1
            && self
                .text
                .trim()
                .eq_ignore_ascii_case(self.links[0].title.trim())
    }
}

pub fn prepare_canvas_body(html: &str, course: &str) -> PreparedCanvasBody {
    let (body, links) = prepare_canvas_page(html, course);
    let text = notice_text(&body);
    PreparedCanvasBody {
        body: body.into(),
        links,
        text,
    }
}

/// Canvas page bodies render through a text view that opens every link in the
/// system browser. File links should instead open the app's document reader,
/// so collect them, and remove their anchors from the body. The link text stays
/// in place as plain text and the caller adds a native file action.
pub fn prepare_canvas_page(html: &str, course: &str) -> (String, Vec<CanvasFileLink>) {
    // Most pages have no file links. Keep their HTML byte-for-byte rather than
    // round-tripping it through the parser and serializer.
    if !html.contains("files/") {
        return (html.to_owned(), Vec::new());
    }
    let dom = parse_document(RcDom::default(), Default::default()).one(html);
    let mut links = Vec::new();
    strip_file_links(&dom.document, course, &mut links);
    let mut bytes = Vec::new();
    let body = serialize(
        &mut bytes,
        &SerializableHandle::from(dom.document.clone()),
        SerializeOpts::default(),
    )
    .ok()
    .and_then(|()| String::from_utf8(bytes).ok())
    .unwrap_or_else(|| html.to_owned());
    (body, links)
}

fn strip_file_links(parent: &Handle, course: &str, links: &mut Vec<CanvasFileLink>) {
    let mut index = 0;
    loop {
        let child = parent.children.borrow().get(index).cloned();
        let Some(child) = child else {
            break;
        };
        if let Some((course, file_id, title)) = file_link(&child, course) {
            let children: Vec<_> = child.children.borrow().iter().cloned().collect();
            for node in &children {
                node.parent.set(Some(Rc::downgrade(parent)));
            }
            parent.children.borrow_mut().splice(index..=index, children);
            if !links
                .iter()
                .any(|link| link.course == course && link.file_id == file_id)
            {
                links.push(CanvasFileLink {
                    course,
                    file_id,
                    title,
                });
            }
            // Process whatever was inside the anchor at the same index.
            continue;
        }
        strip_file_links(&child, course, links);
        index += 1;
    }
}

fn file_link(node: &Handle, current_course: &str) -> Option<(String, String, String)> {
    let NodeData::Element { name, attrs, .. } = &node.data else {
        return None;
    };
    if name.local.as_ref() != "a" {
        return None;
    }
    let href = attrs
        .borrow()
        .iter()
        .find(|attribute| attribute.name.local.as_ref() == "href")?
        .value
        .to_string();
    let (course, file_id) = canvas_file_target(&href, current_course)?;
    let mut title = String::new();
    collect_text(node, &mut title);
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    Some((
        course,
        file_id,
        if title.is_empty() {
            "Attached file".into()
        } else {
            title
        },
    ))
}

/// Extract numeric course/file identifiers from Canvas file URLs. Relative
/// URLs and absolute links on the configured Canvas host are both accepted;
/// other schemes and non-file pages are left alone.
fn canvas_file_target(href: &str, current_course: &str) -> Option<(String, String)> {
    let url = url::Url::parse(href)
        .or_else(|_| url::Url::parse("https://canvas.invalid").and_then(|base| base.join(href)))
        .ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let segments: Vec<_> = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect();
    let mut course = current_course.to_owned();
    let mut file = None;
    let mut index = 0;
    while index + 1 < segments.len() {
        match segments[index] {
            "courses" if numeric(segments[index + 1]) => {
                course = segments[index + 1].to_owned();
                index += 2;
            }
            "files" if numeric(segments[index + 1]) => {
                file = Some(segments[index + 1].to_owned());
                index += 2;
            }
            _ => index += 1,
        }
    }
    Some((course, file?))
}

fn numeric(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
}

fn collect_text(node: &Handle, out: &mut String) {
    if let NodeData::Text { contents } = &node.data {
        out.push_str(&contents.borrow());
    }
    for child in node.children.borrow().iter() {
        collect_text(child, out);
    }
}

pub fn notice_text(html: &str) -> String {
    fn visit(node: &Handle, out: &mut String) {
        if let NodeData::Element { name, attrs, .. } = &node.data {
            let tag = name.local.as_ref();
            if matches!(tag, "script" | "style" | "template" | "noscript") {
                return;
            }
            if attrs.borrow().iter().any(|a| {
                let key = a.name.local.as_ref();
                key == "hidden"
                    || (key == "aria-hidden" && a.value.as_ref() == "true")
                    || (key == "style"
                        && a.value.split(';').any(|s| {
                            s.split_once(':').is_some_and(|(k, v)| {
                                matches!(
                                    (
                                        k.trim().to_ascii_lowercase().as_str(),
                                        v.trim().to_ascii_lowercase().as_str()
                                    ),
                                    ("display", "none") | ("visibility", "hidden")
                                )
                            })
                        }))
            }) {
                return;
            }
            if matches!(tag, "br" | "p" | "div" | "li") {
                out.push(' ');
            }
        }
        if let NodeData::Text { contents } = &node.data {
            out.push_str(&contents.borrow());
        }
        for child in node.children.borrow().iter() {
            visit(child, out);
        }
        if let NodeData::Element { name, .. } = &node.data
            && matches!(name.local.as_ref(), "p" | "div" | "li")
        {
            out.push(' ');
        }
    }
    let dom = parse_document(RcDom::default(), Default::default()).one(html);
    let mut text = String::new();
    visit(&dom.document, &mut text);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canvas_lock_notice_excludes_hidden_browser_controls() {
        let html = "This assignment is part of <b>Module 9</b> and hasn&#39;t been unlocked yet.<br><div class=\"spinner\"></div><a style=\"display: none;\" href=\"/private\">Hidden instructions</a><a hidden>&nbsp;</a>";
        assert_eq!(
            notice_text(html),
            "This assignment is part of Module 9 and hasn't been unlocked yet."
        );
    }
    #[test]
    fn handles_entities_plain_text_and_malformed_html() {
        assert_eq!(
            notice_text("<p>A &amp; B</p><p>C&nbsp;D <b>unfinished"),
            "A & B C D unfinished"
        );
        assert_eq!(notice_text("Available tomorrow."), "Available tomorrow.");
        assert_eq!(
            notice_text("<script>bad()</script><div style='DISPLAY: none'>hidden</div>Visible"),
            "Visible"
        );
    }
    #[test]
    fn canvas_file_links_leave_the_body_and_become_reader_actions() {
        let html = r#"<p>Read <a href="/courses/4/files/77/download?verifier=x">Syllabus.pdf</a> before class.</p>"#;
        let (body, links) = prepare_canvas_page(html, "4");
        assert_eq!(
            links,
            vec![CanvasFileLink {
                course: "4".into(),
                file_id: "77".into(),
                title: "Syllabus.pdf".into(),
            }]
        );
        assert!(!body.contains("<a "));
        assert!(body.contains("Syllabus.pdf"));
        assert!(body.contains("before class"));
    }
    #[test]
    fn absolute_and_relative_canvas_file_urls_are_recognized_once() {
        for href in [
            "https://canvas.example.edu/courses/9/files/12/preview",
            "/courses/9/files/12?wrap=1",
            "files/12/download",
        ] {
            let html = format!(r#"<a href="{href}">Reading</a><a href="{href}">Reading again</a>"#);
            let (body, links) = prepare_canvas_page(&html, "3");
            assert_eq!(links.len(), 1, "{href}");
            assert_eq!(
                links[0].course,
                if href.contains("/courses/") { "9" } else { "3" }
            );
            assert_eq!(links[0].file_id, "12");
            assert!(!body.contains("<a "));
        }
    }
    #[test]
    fn non_canvas_and_non_file_links_stay_as_links() {
        let html = r#"<a href="https://example.test/paper.pdf">Remote paper</a><a href="/courses/4/pages/week-1">Week 1</a><a href="mailto:teacher@example.test">Email</a>"#;
        let (body, links) = prepare_canvas_page(html, "4");
        assert!(links.is_empty());
        assert_eq!(body.matches("<a ").count(), 3);
    }
}
