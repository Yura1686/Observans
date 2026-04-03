const HTML_TEMPLATE: &str = include_str!("../assets/index.html");
const STYLES: &str = include_str!("../assets/styles.css");
const SCRIPT: &str = include_str!("../assets/app.js");

pub fn render_index_html() -> String {
    HTML_TEMPLATE
        .replace("{{STYLES}}", STYLES)
        .replace("{{SCRIPT}}", SCRIPT)
}

#[cfg(test)]
mod tests {
    use super::render_index_html;

    #[test]
    fn keeps_old_ui_contract_ids() {
        let html = render_index_html();

        assert!(html.contains("id=\"record-btn\""));
        assert!(html.contains("id=\"stop-btn\""));
        assert!(html.contains("id=\"save-btn\""));
        assert!(html.contains("id=\"stream-stage\""));
        assert!(html.contains("id=\"fullscreen-btn\""));
        assert!(html.contains("id=\"battery-fill\""));
    }
}
