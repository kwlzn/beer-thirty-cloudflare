use regex::Regex;

#[derive(Debug, Clone)]
pub struct Element {
    tag_name: String,
    attributes: Vec<(String, String)>,
    content: String,
}

impl Element {
    pub fn get_attr(&self, name: &str) -> Option<String> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    }

    pub fn get_content(&self) -> &str {
        &self.content
    }
}

pub fn find_elements_by_class(html: &str, class_name: &str) -> Vec<Element> {
    if html.is_empty() || class_name.is_empty() {
        return Vec::new();
    }

    let mut elements = Vec::new();

    // Match any opening tag with any attributes
    let open_tag_re = Regex::new(r#"<([a-zA-Z][a-zA-Z0-9]*)\s*([^>]*)>"#).unwrap();
    let class_re = Regex::new(r#"class\s*=\s*['"]([^'"]*?)['"]"#).unwrap();
    let attr_re = Regex::new(r#"([a-zA-Z][a-zA-Z0-9-]*)\s*=\s*['"]([^'"]*?)['"]"#).unwrap();

    let mut search_pos = 0;
    while let Some(tag_match) = open_tag_re.find(&html[search_pos..]) {
        // Get the absolute position in the original string
        // let abs_pos = search_pos + tag_match.start();

        // Get captures from the current position
        if let Some(cap) = open_tag_re.captures(&html[search_pos..]) {
            let tag_name = cap.get(1).unwrap().as_str();
            let attrs_str = cap.get(2).unwrap().as_str();

            // Check if this element has the target class
            if let Some(class_cap) = class_re.captures(attrs_str) {
                let class_value = class_cap.get(1).unwrap().as_str();
                let has_class = class_value
                    .split_whitespace()
                    .any(|class| class == class_name);

                if has_class {
                    // Find matching closing tag
                    let tag_end = search_pos + tag_match.end();
                    let closing_tag = format!("</{}>", tag_name);
                    let open_tag = format!("<{}", tag_name);
                    let mut depth = 1;
                    let mut content_end = tag_end;

                    // Find the matching closing tag considering nested elements
                    let mut pos = tag_end;
                    while pos < html.len() {
                        let rest = &html[pos..];
                        let next_open = rest.find(&open_tag);
                        let next_close = rest.find(&closing_tag);

                        match (next_open, next_close) {
                            // Found both open and close tags
                            (Some(o), Some(c)) => {
                                if o < c {
                                    depth += 1;
                                    pos += o + 1;
                                } else {
                                    depth -= 1;
                                    if depth == 0 {
                                        content_end = pos + c;
                                        break;
                                    }
                                    pos += c + closing_tag.len();
                                }
                            },
                            // Only found closing tag
                            (None, Some(c)) => {
                                depth -= 1;
                                if depth == 0 {
                                    content_end = pos + c;
                                    break;
                                }
                                pos += c + closing_tag.len();
                            },
                            // No more tags found
                            _ => break,
                        }
                    }

                    if depth == 0 {
                        // Parse attributes
                        let mut attributes = Vec::new();
                        for attr_cap in attr_re.captures_iter(attrs_str) {
                            if let (Some(key), Some(value)) = (attr_cap.get(1), attr_cap.get(2)) {
                                attributes.push((
                                    key.as_str().to_string(),
                                    value.as_str().to_string(),
                                ));
                            }
                        }

                        // Extract content
                        let content = html[tag_end..content_end].trim().to_string();

                        elements.push(Element {
                            tag_name: tag_name.to_string(),
                            attributes,
                            content,
                        });

                        search_pos = content_end + closing_tag.len();
                        continue;
                    }
                }
            }
        }
        search_pos += tag_match.end();
    }

    elements
}

pub fn find_first_anchor(html: &str) -> Option<Element> {
    if html.is_empty() {
        return None;
    }

    let open_re = Regex::new(r#"<a\s*([^>]*)>"#).unwrap();
    let close_re = Regex::new(r#"</a\s*>"#).unwrap();
    let attr_re = Regex::new(r#"([a-zA-Z][a-zA-Z0-9-]*)\s*=\s*['"]([^'"]*?)['"]"#).unwrap();

    if let Some(open_cap) = open_re.captures(html) {
        let full_open = open_cap.get(0).unwrap();
        let attrs_str = open_cap.get(1).map_or("", |m| m.as_str());
        let after_open = &html[full_open.end()..];

        if let Some(close_match) = close_re.find(after_open) {
            let content = &after_open[..close_match.start()];

            let mut attributes = Vec::new();
            for attr_cap in attr_re.captures_iter(attrs_str) {
                if let (Some(key), Some(value)) = (attr_cap.get(1), attr_cap.get(2)) {
                    attributes.push((
                        key.as_str().to_string(),
                        value.as_str().to_string(),
                    ));
                }
            }

            return Some(Element {
                tag_name: "a".to_string(),
                attributes,
                content: content.trim().to_string(),
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_elements_by_class() {
        let html = r#"
            <div class="beer-item foo">
                <a href="/beer/123">Some Beer</a>
                <div class="caps bar" data-rating="4.2">Rating</div>
            </div>
        "#;

        let elements = find_elements_by_class(html, "beer-item");
        assert_eq!(elements.len(), 1);
        assert!(elements[0].content.contains("Some Beer"));

        let caps = find_elements_by_class(&elements[0].content, "caps");
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].get_attr("data-rating").unwrap(), "4.2");
    }

    #[test]
    fn test_find_first_anchor() {
        let html = r#"<div><a href="/beer/123">Some Beer</a></div>"#;
        let anchor = find_first_anchor(html);
        assert!(anchor.is_some());
        assert_eq!(anchor.unwrap().get_attr("href").unwrap(), "/beer/123");
    }

    #[test]
    fn test_find_first_anchor_with_whitespace() {
        let html = r#"
            <div>
                <a href="/beer/123">
                    Some Beer
                </a>
            </div>
        "#;
        let anchor = find_first_anchor(html);
        assert!(anchor.is_some());
        assert_eq!(anchor.unwrap().get_attr("href").unwrap(), "/beer/123");
    }

    #[test]
    fn test_nested_elements() {
        let html = r#"
            <div class="outer foo">
                <div class="inner bar">
                    <div class="caps baz" data-rating="4.2">Rating</div>
                </div>
            </div>
        "#;

        let inner_elements = find_elements_by_class(html, "inner");
        assert_eq!(inner_elements.len(), 1);

        let caps = find_elements_by_class(&inner_elements[0].content, "caps");
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].get_attr("data-rating").unwrap(), "4.2");
    }

    #[test]
    fn test_multi_class() {
        let html = r#"<div class="foo bar caps baz" data-rating="4.2">Rating</div>"#;
        let elements = find_elements_by_class(html, "caps");
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].get_attr("data-rating").unwrap(), "4.2");
    }

    #[test]
    fn test_empty_input() {
        assert!(find_elements_by_class("", "test").is_empty());
        assert!(find_first_anchor("").is_none());
    }

    #[test]
    fn test_no_matches() {
        assert!(find_elements_by_class("<div>test</div>", "nonexistent").is_empty());
        assert!(find_first_anchor("<div>test</div>").is_none());
    }
}