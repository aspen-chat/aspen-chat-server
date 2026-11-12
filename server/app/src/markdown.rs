//! The Markdown dialect messages are written in, as the server reads them: CommonMark with
//! GitHub's tables, strikethrough, and the rest of its extensions, which is what the client
//! renders (`react-markdown` with `remark-gfm`). Whatever the server finds in a message (links
//! to preview, tags) it finds where the client shows text, and never in code.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// The events of `content` as a message.
pub fn parser(content: &str) -> Parser<'_> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_GFM);
    Parser::new_ext(content, options)
}

/// `text` written so that a message shows exactly it: a backslash before each character
/// Markdown could read as markup.
pub fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Gives `each` every run of `content` that a message shows as text, never code of any kind:
/// what stands between one piece of markup and the next, its line breaks as `\n`.
pub fn text_runs(content: &str, mut each: impl FnMut(&str)) {
    // A run of text may come as several events; it is read whole, up to the next markup.
    let mut run = String::new();
    let mut code_depth = 0u32;
    for event in parser(content) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                each(&std::mem::take(&mut run));
                code_depth += 1;
            }
            Event::End(TagEnd::CodeBlock) => code_depth = code_depth.saturating_sub(1),
            _ if code_depth > 0 => {}
            Event::Text(text) => run.push_str(&text),
            Event::SoftBreak | Event::HardBreak => run.push('\n'),
            _ => each(&std::mem::take(&mut run)),
        }
    }
    each(&run);
}
