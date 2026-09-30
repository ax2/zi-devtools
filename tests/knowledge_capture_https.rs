//! Optional public-web smoke test; no account or browser data is sent.
use zi_devtools::knowledge_capture;

#[test]
#[ignore = "requires direct public HTTPS access to devtools.zicode.com"]
fn fetches_a_public_page_without_browser_credentials() {
    let preview = knowledge_capture::preview_https("https://devtools.zicode.com/").unwrap();
    assert!(preview.title.contains("Zi DevTools"));
    assert!(preview.body.contains("Windows"));
    assert_eq!(preview.source, "https://devtools.zicode.com/");
}
