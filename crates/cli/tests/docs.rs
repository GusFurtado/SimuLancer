//! The documentation's complete models (YAML blocks that start a model
//! file, in `docs/guides/`) must load: a guide whose example doesn't work is
//! worse than none.

use std::fs;
use std::path::Path;

#[test]
fn guide_models_load() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/guides");
    let mut checked = 0;
    let mut failures = Vec::new();
    let mut pages: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    pages.sort();
    for page in pages
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
    {
        let text = fs::read_to_string(page).unwrap();
        for block in text.split("```yaml\n").skip(1) {
            let yaml = block.split("```").next().unwrap();
            if !yaml.contains("format: simulancer/0.1") {
                continue;
            }
            checked += 1;
            if let Err(e) = simulancer_model::load_str(yaml) {
                let messages: Vec<String> =
                    e.diagnostics().iter().map(|d| d.message.clone()).collect();
                failures.push(format!("{}: {}", page.display(), messages.join("; ")));
            }
        }
    }
    assert!(checked >= 7, "found only {checked} models in docs/guides");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
