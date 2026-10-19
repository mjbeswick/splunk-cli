use crate::types::DashboardPanel;

/// Parse panel titles + SPL queries out of a Splunk Simple XML dashboard
/// definition. Falls back gracefully (returns no panels) for JSON-format
/// (Dashboard Studio) dashboards, which don't use this XML shape.
pub fn parse_panels(xml: &str) -> Vec<DashboardPanel> {
    let doc = match roxmltree::Document::parse(xml) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut panels = Vec::new();

    for panel_node in doc.descendants().filter(|n| n.has_tag_name("panel")) {
        let title = panel_node
            .descendants()
            .find(|n| n.has_tag_name("title"))
            .and_then(|n| n.text())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("(untitled)")
            .to_string();

        let search = panel_node
            .descendants()
            .find(|n| n.has_tag_name("query"))
            .and_then(|n| n.text())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("(no query)")
            .to_string();

        panels.push(DashboardPanel { title, search });
    }

    panels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_panel() {
        let xml = r#"
            <dashboard>
              <row>
                <panel>
                  <title>Errors over time</title>
                  <chart>
                    <search>
                      <query>index=ghs_ppe level=ERROR | timechart count</query>
                    </search>
                  </chart>
                </panel>
              </row>
            </dashboard>
        "#;
        let panels = parse_panels(xml);
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].title, "Errors over time");
        assert_eq!(panels[0].search, "index=ghs_ppe level=ERROR | timechart count");
    }

    #[test]
    fn handles_missing_title_or_query() {
        let xml = r#"<dashboard><row><panel><chart/></panel></row></dashboard>"#;
        let panels = parse_panels(xml);
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].title, "(untitled)");
        assert_eq!(panels[0].search, "(no query)");
    }

    #[test]
    fn handles_invalid_xml_gracefully() {
        let panels = parse_panels("not xml at all {}");
        assert!(panels.is_empty());
    }

    #[test]
    fn handles_multiple_panels() {
        let xml = r#"
            <dashboard>
              <row>
                <panel><title>A</title><chart><search><query>q1</query></search></chart></panel>
                <panel><title>B</title><chart><search><query>q2</query></search></chart></panel>
              </row>
            </dashboard>
        "#;
        let panels = parse_panels(xml);
        assert_eq!(panels.len(), 2);
        assert_eq!(panels[0].title, "A");
        assert_eq!(panels[1].title, "B");
    }
}
