fn main() {
    let web_dist = std::path::Path::new("web/dist");
    if !web_dist.exists() {
        if let Err(e) = std::fs::create_dir_all(web_dist) {
            eprintln!("cargo:warning=Failed to create web/dist directory: {}", e);
        }
    }
    let index_html = web_dist.join("index.html");
    if !index_html.exists() {
        let placeholder = r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><title>Matrix Bot</title></head>
<body>
<div style="font-family: sans-serif; padding: 2rem; text-align: center;">
  <h2>Matrix Bot</h2>
  <p>Frontend assets not yet compiled. Run <code>npm --prefix frontend run build</code> to build the web UI.</p>
</div>
</body>
</html>"#;
        if let Err(e) = std::fs::write(&index_html, placeholder) {
            eprintln!(
                "cargo:warning=Failed to write placeholder index.html: {}",
                e
            );
        }
    }
}
