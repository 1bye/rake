//! Preserve authored favicon links and install the shared default without overwriting assets.

use super::Result;
use std::{fs, io::Write, path::Path};

pub(super) fn install(site: &Path) -> Result<()> {
    let index = site.join("index.html");

    let document = fs::read_to_string(&index)?
        .replace("\r\n", "\n")
        .replace('\r', "\n");

    let (has_icon, insertion) = scan(&document);

    if has_icon {
        return Ok(());
    }

    let insertion = insertion.ok_or("Web document requires an html or head element")?;
    let data = include_bytes!("../../scripts/default-icon.svg");
    let assets = site.join("assets");

    if !assets.exists() {
        fs::create_dir(&assets)?;
    }

    let icon = assets.join("rake-icon.svg");

    if !(icon.is_file() && !icon.is_symlink() && fs::read(&icon)? == data) {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&icon)?
            .write_all(data)?;
    }

    let link = "<link rel=\"icon\" type=\"image/svg+xml\" href=\"/assets/rake-icon.svg\">";

    fs::write(
        index,
        format!("{}{link}{}", &document[..insertion], &document[insertion..]),
    )?;

    Ok(())
}

// Use the installed HTML tokenizer for real tags/attributes (including entities),
// but retain source offsets: serializing an HTML tree would rewrite authored bytes.
fn scan(document: &str) -> (bool, Option<usize>) {
    use html5ever::{
        buffer_queue::BufferQueue,
        tendril::StrTendril,
        tokenizer::{
            TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts, states::RawKind,
        },
    };

    struct Sink {
        icon: bool,
        head: bool,
        html: bool,
    }

    impl TokenSink for Sink {
        type Handle = ();

        fn process_token(&mut self, token: Token, _line: u64) -> TokenSinkResult<()> {
            if let Token::TagToken(tag) = token
                && tag.kind == TagKind::StartTag
            {
                match tag.name.as_ref() {
                    "link" => {
                        if tag.attrs.iter().any(|attribute| {
                            attribute.name.local.as_ref() == "rel"
                                && attribute
                                    .value
                                    .split_whitespace()
                                    .any(|word| word.eq_ignore_ascii_case("icon"))
                        }) {
                            self.icon = true;
                        }
                    }
                    "head" => self.head = true,
                    "html" => self.html = true,
                    "script" => return TokenSinkResult::RawData(RawKind::ScriptData),
                    "style" => return TokenSinkResult::RawData(RawKind::Rawtext),
                    _ => {}
                }
            }

            TokenSinkResult::Continue
        }
    }
    let mut tokenizer = Tokenizer::new(
        Sink {
            icon: false,
            head: false,
            html: false,
        },
        TokenizerOpts::default(),
    );

    let mut input = BufferQueue::default();
    let mut insertion = None;
    let mut offset = 0;

    // A start tag can only complete at `>`; chunk there to retain its source offset.
    for chunk in document.split_inclusive('>') {
        offset += chunk.len();
        input.push_back(StrTendril::from_slice(chunk));
        let _ = tokenizer.feed(&mut input);

        if std::mem::take(&mut tokenizer.sink.head)
            || (std::mem::take(&mut tokenizer.sink.html) && insertion.is_none())
        {
            insertion = Some(offset);
        }
    }

    tokenizer.end();

    (tokenizer.sink.icon, insertion)
}

#[cfg(test)]
mod tests {
    //! Translated from the five test-web-deployment.py favicon contracts.
    use super::*;

    #[test]
    fn default_icon_is_shared_and_installation_is_idempotent() {
        let root = tempfile::tempdir().unwrap();

        let original =
            "<!doctype html><html><head><title>App</title></head><body>app</body></html>";

        let index = root.path().join("index.html");

        fs::write(&index, original).unwrap();
        install(root.path()).unwrap();
        let installed = fs::read_to_string(&index).unwrap();
        let link = "<link rel=\"icon\" type=\"image/svg+xml\" href=\"/assets/rake-icon.svg\">";

        assert_eq!(installed.replace(link, ""), original);
        assert_eq!(
            fs::read(root.path().join("assets/rake-icon.svg")).unwrap(),
            include_bytes!("../../../rake-build/assets/default-icon.svg")
        );
        install(root.path()).unwrap();
        assert_eq!(fs::read_to_string(&index).unwrap(), installed);
        fs::write(&index, original).unwrap();
        install(root.path()).unwrap();
        assert_eq!(fs::read_to_string(&index).unwrap(), installed);
    }

    #[test]
    fn authored_favicons_including_empty_icons_are_preserved() {
        for link in [
            "<link rel=\"icon\" href=\"./custom.svg\">",
            "<LINK REL='shortcut ICON' HREF='custom.ico'>",
            "<link rel=icon href=\"data:,\">",
            "<link href=\"custom.png\" rel=\"icon alternate\"/>",
        ] {
            let root = tempfile::tempdir().unwrap();
            let original = format!("<html><head>{link}</head><body></body></html>");

            fs::write(root.path().join("index.html"), &original).unwrap();
            install(root.path()).unwrap();
            assert_eq!(
                fs::read_to_string(root.path().join("index.html")).unwrap(),
                original
            );
            assert!(!root.path().join("assets").exists());
        }
    }

    #[test]
    fn comments_and_script_strings_do_not_count_as_favicon_links() {
        let root = tempfile::tempdir().unwrap();

        fs::write(root.path().join("index.html"), "<!-- <head><link rel=\"icon\" href=\"comment.svg\"> --><html>\n<head><script>const example = '<link rel=\"icon\" href=\"script.svg\">';</script></head></html>").unwrap();
        install(root.path()).unwrap();
        assert!(
            fs::read_to_string(root.path().join("index.html"))
                .unwrap()
                .contains("<head><link rel=\"icon\" type=\"image/svg+xml\"")
        );
        assert!(root.path().join("assets/rake-icon.svg").is_file());
    }

    #[test]
    fn implicit_head_is_supported() {
        let root = tempfile::tempdir().unwrap();

        fs::write(
            root.path().join("index.html"),
            "<html lang=\"en\"><meta charset=\"utf-8\"><body>App</body></html>",
        )
        .unwrap();
        install(root.path()).unwrap();
        assert!(
            fs::read_to_string(root.path().join("index.html"))
                .unwrap()
                .contains("<html lang=\"en\"><link rel=\"icon\"")
        );
    }

    #[test]
    fn generated_icon_does_not_overwrite_an_application_file() {
        let root = tempfile::tempdir().unwrap();
        let original = "<html><head></head></html>";

        fs::write(root.path().join("index.html"), original).unwrap();
        fs::create_dir(root.path().join("assets")).unwrap();
        fs::write(root.path().join("assets/rake-icon.svg"), "app-owned").unwrap();
        assert!(install(root.path()).is_err());
        assert_eq!(
            fs::read_to_string(root.path().join("index.html")).unwrap(),
            original
        );
        assert_eq!(
            fs::read_to_string(root.path().join("assets/rake-icon.svg")).unwrap(),
            "app-owned"
        );
    }
}
