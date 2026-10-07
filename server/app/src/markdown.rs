//! The Markdown dialect messages are written in, as the server reads them: CommonMark with
//! GitHub's tables, strikethrough, and the rest of its extensions, which is what the client
//! renders (`react-markdown` with `remark-gfm`). Whatever the server finds in a message (links
//! to preview, tags) it finds where the client shows text, and never in code.

use pulldown_cmark::{Options, Parser};

/// The events of `content` as a message.
pub fn parser(content: &str) -> Parser<'_> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_GFM);
    Parser::new_ext(content, options)
}
