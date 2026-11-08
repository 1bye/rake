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
