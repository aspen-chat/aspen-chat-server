//! Where a message's spoilers and code are, so a notification hides what the message hides.
//!
//! A spoiler is whatever the client renders as one, and the client finds them in its Markdown
//! syntax tree (`client/packages/app/src/features/messages/remarkSpoilers.ts`), so this reads
//! the same tree out of pulldown-cmark's events and looks through it the same way:
//!
//! - A spoiler is `||text||` or `>!text!<`, its markers in the text of one parent (a
//!   paragraph, a heading, a table cell, emphasis): in one run of text, where what they hold
//!   must not be blank, or in two with anything between them, code included.
//! - A marker is two characters written as themselves. One written with a backslash (`\|`)
//!   or as a character reference (`&#124;`) is text and opens and closes nothing, and code is
//!   not text, so a marker in code is none. A table cell is the exception for the pipe, which
//!   a cell can hold only as `\|`: there `\|\|text\|\|` is a spoiler.
//! - A link named by its author's words, a picture, and a reference's definition are the text
//!   they were written as, as the client shows them, backslashes and all, and the markers
//!   that text holds count.
//! - Openers are tried from left to right, each closing at the nearest closer after it
//!   ([`opening`]). One that makes no spoiler leaves the text after it to be read for the
//!   next: after `|| ||`, whose markers hold only blank text, from the end of its closer, and
//!   after `||||`, whose markers hold nothing, from the second of the two, which may open.
//! - A quote whose text starts with `!` and holds `!<` is a `>!` spoiler, its `>` the quote's.
//! - A message nested too deeply to draw is read as the client shows it: as text with no
//!   Markdown, or, when its lines open too many quotes and lists to parse, with no spoilers.
//!
//! `spec/spoiler_vectors.json` holds texts this and the client read alike. Change the two
//! together.
//!
//! The two parsers do not build the same tree of every text, and where they differ a spoiler
//! may be shown here as it was written:
//!
//! - What pulldown-cmark reports nothing of is not read: a definition of a label defined
//!   already, and a table cell past its table's width. The client shows both.
//! - A table too large for the client to draw is code to it and a table here.

use pulldown_cmark::{Event, LinkType, Tag, TagEnd};
use std::ops::Range;

/// A spelling of spoiler: what opens one and what closes it.
struct Marker {
    open: &'static str,
    close: &'static str,
}

/// Discord's and Reddit's, in the order the client tries them.
const MARKERS: [Marker; 2] = [
    Marker {
        open: "||",
        close: "||",
    },
    Marker {
        open: ">!",
        close: "!<",
    },
];

/// How deeply the client lets a message's syntax tree nest (`MAX_NESTING` in its
/// `markdownLimits.ts`) before it shows the message as text with no Markdown.
const MAX_NESTING: usize = 32;

/// Where a message's code and spoilers are, as byte ranges of its text, each list in order
/// with no range inside another.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Reading {
    /// Code spans and blocks, markup included.
    pub code: Vec<Range<usize>>,
    /// Spoilers, markers included.
    pub spoilers: Vec<Range<usize>>,
}

/// Reads `content` as a message.
pub(super) fn read(content: &str) -> Reading {
    let tree = Tree::read(content);
    Reading {
        code: tree.code,
        spoilers: outermost(tree.spoilers),
    }
}

/// The spoilers that lie in no other, in order.
fn outermost(mut spoilers: Vec<Spoiler>) -> Vec<Range<usize>> {
    spoilers.sort_by_key(|spoiler| spoiler.whole.start);
    let mut kept: Vec<Range<usize>> = Vec::new();
    for spoiler in spoilers {
        if kept
            .last()
            .is_none_or(|last| last.end <= spoiler.whole.start)
        {
            kept.push(spoiler.whole);
        }
    }
    kept
}

/// One spoiler, as byte ranges of the message's text.
struct Spoiler {
    /// From the start of its opening marker to the end of its closing one.
    whole: Range<usize>,
    /// What stands between the markers.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the tests tell markers from what they hold by it")
    )]
    inner: Range<usize>,
}

/// A stretch of a run of text that came from one stretch of the source.
struct Piece {
    /// Where the stretch starts in the run.
    at: usize,
    source: Range<usize>,
    /// Whether the stretch is its source byte for byte. One that is not (an entity, a line
    /// break) is placed in the source only as a whole.
    verbatim: bool,
}

/// How a stretch of text was written, which decides whether its characters can spell a
/// marker.
#[derive(Clone, Copy, PartialEq)]
enum Written {
    /// As itself, byte for byte.
    Plain,
    /// As itself after a backslash that escapes its first character.
    Escaped,
    /// As something else: a character reference, a line break.
    Otherwise,
}

/// The characters the markers are spelled with.
const MARKER_CHARACTERS: [char; 4] = ['|', '>', '!', '<'];
/// What stands in [`Text::markers`] for a character that opens and closes nothing.
const LITERAL: &str = "\0";

/// A run of text as the reader is shown it, with where each byte of it was written.
#[derive(Default)]
struct Text {
    value: String,
    /// `value` as markers are looked for in it, byte for byte: a character that can spell a
    /// marker and was not written as itself is U+0000 here.
    markers: String,
    pieces: Vec<Piece>,
}

impl Text {
    fn push(&mut self, value: &str, source: Range<usize>, written: Written) {
        self.pieces.push(Piece {
            at: self.value.len(),
            source,
            verbatim: written != Written::Otherwise,
        });
        self.value.push_str(value);
        let literal = match written {
            Written::Plain => 0,
            Written::Escaped => value.chars().next().map_or(0, char::len_utf8),
            Written::Otherwise => value.len(),
        };
        let (literal, plain) = value.split_at(literal);
        self.markers
            .push_str(&literal.replace(MARKER_CHARACTERS, LITERAL));
        self.markers.push_str(plain);
    }

    /// Puts `value` before the rest.
    fn prefix(&mut self, value: &str, source: Range<usize>) {
        for piece in &mut self.pieces {
            piece.at += value.len();
        }
        self.pieces.insert(
            0,
            Piece {
                at: 0,
                source,
                verbatim: true,
            },
        );
        self.value.insert_str(0, value);
        self.markers.insert_str(0, value);
    }

    /// The index of the piece holding the byte at `at`.
    fn piece(&self, at: usize) -> usize {
        self.pieces.partition_point(|piece| piece.at <= at) - 1
    }

    /// Where in `content` the byte at `at` starts, with the backslash that escapes it.
    fn source_start(&self, at: usize, content: &str) -> usize {
        let index = self.piece(at);
        let piece = &self.pieces[index];
        if piece.verbatim && at > piece.at {
            return piece.source.start + (at - piece.at);
        }
        let start = piece.source.start;
        let before = index
            .checked_sub(1)
            .map_or(0, |index| self.pieces[index].source.end);
        // An escaped character is a piece of its own that starts after its backslash.
        if start > before && content.as_bytes()[start - 1] == b'\\' {
            start - 1
        } else {
            start
        }
    }

    /// Where in the source the one-byte character at `at` ends.
    fn source_end(&self, at: usize) -> usize {
        let piece = &self.pieces[self.piece(at)];
        if piece.verbatim {
            piece.source.start + (at - piece.at) + 1
        } else {
            piece.source.end
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Paragraph,
    Quote,
    /// A list item, whose text pulldown-cmark gives with no paragraph around it when its list
    /// has no blank lines.
    Item,
    /// What stands in a line of text and holds more of it: emphasis, an autolink.
    Inline,
    /// A table cell, where a pipe is written `\|`.
    Cell,
    Other,
}

/// A node of the syntax tree that holds others.
struct Parent {
    kind: Kind,
    /// Where it starts in the source.
    start: usize,
    /// Whether it is a paragraph pulldown-cmark left out, around a list item's text.
    implied: bool,
    children: Vec<Child>,
}

enum Child {
    Text(Text),
    Parent(Parent),
    /// Anything that is neither text nor holds any: code, a line break, HTML.
    Leaf(
        #[cfg_attr(
            not(test),
            expect(dead_code, reason = "the tests place what is not text by it")
        )]
        Range<usize>,
    ),
}

/// A message's syntax tree, read through.
struct Tree {
    root: Parent,
    code: Vec<Range<usize>>,
    /// Every spoiler, those inside others too.
    spoilers: Vec<Spoiler>,
}

impl Tree {
    fn read(content: &str) -> Self {
        if opens_too_deeply(content) {
            return Tree::flat(content);
        }
        let mut tree = Builder::build(content).unwrap_or_else(|| Tree::flat(content));
        find(&mut tree.root, content, &mut tree.spoilers);
        tree
    }

    /// `content` as one paragraph of text with no Markdown.
    fn flat(content: &str) -> Self {
        let mut text = Text::default();
        text.push(content, 0..content.len(), Written::Plain);
        let mut paragraph = Parent::new(Kind::Paragraph, 0);
        paragraph.children.push(Child::Text(text));
        let mut root = Parent::new(Kind::Other, 0);
        root.children.push(Child::Parent(paragraph));
        Tree {
            root,
            code: Vec::new(),
            spoilers: Vec::new(),
        }
    }
}

impl Parent {
    fn new(kind: Kind, start: usize) -> Self {
        Parent {
            kind,
            start,
            implied: false,
            children: Vec::new(),
        }
    }
}

/// Events being passed over, up to the end of the element they are in.
struct Skipping {
    /// How many elements inside that one are open.
    open: usize,
    /// Whether what is passed over counts toward the tree's depth: a link's text does, since
    /// the client measures depth before it turns the link into text.
    counted: bool,
}

/// Builds the tree from pulldown-cmark's events.
struct Builder<'a> {
    content: &'a str,
    /// The parents open, the root first.
    stack: Vec<Parent>,
    code: Vec<Range<usize>>,
    skipping: Option<Skipping>,
    /// Where the last text, code, or break read ended in the source.
    read_to: usize,
}

impl<'a> Builder<'a> {
    /// The tree of `content`, or `None` when it nests deeper than [`MAX_NESTING`].
    fn build(content: &'a str) -> Option<Tree> {
        let mut builder = Builder {
            content,
            stack: vec![Parent::new(Kind::Other, 0)],
            code: Vec::new(),
            skipping: None,
            read_to: 0,
        };
        let events = crate::markdown::parser(content).into_offset_iter();
        // A definition makes no event; the client shows it as the paragraph it was written as.
        let mut definitions: Vec<Range<usize>> = events
            .reference_definitions()
            .iter()
            .map(|(_, definition)| definition.span.clone())
            .collect();
        for (event, range) in events {
            if !builder.take(event, range) {
                return None;
            }
        }
        while builder.stack.len() > 1 {
            builder.close();
        }
        let mut root = builder.stack.pop()?;
        definitions.sort_by_key(|span| span.start);
        for span in definitions {
            let mut text = Text::default();
            text.push(&content[span.clone()], span.clone(), Written::Plain);
            let mut paragraph = Parent::new(Kind::Paragraph, span.start);
            paragraph.children.push(Child::Text(text));
            root.children.push(Child::Parent(paragraph));
        }
        Some(Tree {
            root,
            code: builder.code,
            spoilers: Vec::new(),
        })
    }

    fn top(&mut self) -> &mut Parent {
        self.stack.last_mut().expect("the root is never closed")
    }

    /// Closes the innermost open parent into the one around it.
    fn close(&mut self) {
        if let Some(closed) = self.stack.pop() {
            self.top().children.push(Child::Parent(closed));
        }
    }

    /// Makes ready for something inline: in a list item, inside the paragraph it implies.
    fn inline(&mut self, at: usize) {
        if self.top().kind == Kind::Item {
            let mut paragraph = Parent::new(Kind::Paragraph, at);
            paragraph.implied = true;
            self.stack.push(paragraph);
        }
    }

    /// Makes ready for a block, which ends the paragraph a list item's text implied.
    fn block(&mut self) {
        if self.top().implied {
            self.close();
        }
    }

    /// Whether a node added now, `inside` levels below the innermost open parent, is within
    /// the depth the client draws.
    fn fits(&self, inside: usize) -> bool {
        self.stack.len() + inside <= MAX_NESTING
    }

    /// Adds text an event gave. Its first character was escaped when a backslash stands
    /// right before it that is part of nothing read before: pulldown-cmark starts a new text
    /// after each escape's backslash. In a table cell an escaped pipe is a pipe written the
    /// only way a cell can.
    fn text(&mut self, value: &str, source: Range<usize>) {
        let written = if self.content.get(source.clone()) != Some(value) {
            Written::Otherwise
        } else if source.start > self.read_to
            && self.content.as_bytes()[source.start - 1] == b'\\'
            && !(value.starts_with('|') && self.stack.iter().any(|open| open.kind == Kind::Cell))
        {
            Written::Escaped
        } else {
            Written::Plain
        };
        self.add_text(value, source, written);
    }

    fn add_text(&mut self, value: &str, source: Range<usize>, written: Written) {
        if value.is_empty() {
            return;
        }
        self.inline(source.start);
        self.read_to = source.end;
        let children = &mut self.top().children;
        if !matches!(children.last(), Some(Child::Text(_))) {
            children.push(Child::Text(Text::default()));
        }
        if let Some(Child::Text(text)) = children.last_mut() {
            text.push(value, source, written);
        }
    }

    fn leaf(&mut self, source: Range<usize>) {
        self.read_to = source.end;
        self.top().children.push(Child::Leaf(source));
    }

    /// Adds `event` to the tree. `false` when the tree has grown too deep.
    fn take(&mut self, event: Event<'_>, range: Range<usize>) -> bool {
        if let Some(skipping) = &mut self.skipping {
            let inside = match event {
                Event::Start(_) => {
                    skipping.open += 1;
                    skipping.open
                }
                Event::End(_) if skipping.open == 0 => {
                    self.skipping = None;
                    return true;
                }
                Event::End(_) => {
                    skipping.open -= 1;
                    return true;
                }
                _ => skipping.open + 1,
            };
            return !skipping.counted || self.fits(inside);
        }
        match event {
            Event::Start(tag) => return self.open(tag, range),
            Event::End(end) => {
                if matches!(end, TagEnd::Item) {
                    self.block();
                }
                self.close();
                return true;
            }
            Event::Text(text) => self.text(&text, range),
            Event::SoftBreak => self.text("\n", range),
            Event::Code(_) => {
                self.inline(range.start);
                self.code.push(range.clone());
                self.leaf(range);
            }
            Event::Rule | Event::Html(_) | Event::DisplayMath(_) => {
                self.block();
                self.leaf(range);
            }
            Event::HardBreak
            | Event::InlineHtml(_)
            | Event::InlineMath(_)
            | Event::FootnoteReference(_)
            | Event::TaskListMarker(_) => {
                self.inline(range.start);
                self.leaf(range);
            }
        }
        self.fits(0)
    }

    /// Opens what `tag` starts. `false` when the tree has grown too deep.
    fn open(&mut self, tag: Tag<'_>, range: Range<usize>) -> bool {
        let kind = match tag {
            Tag::CodeBlock(_) | Tag::HtmlBlock | Tag::MetadataBlock(_) => {
                self.block();
                if matches!(tag, Tag::CodeBlock(_)) {
                    self.code.push(range.clone());
                }
                self.leaf(range);
                self.skipping = Some(Skipping {
                    open: 0,
                    counted: false,
                });
                return self.fits(0);
            }
            Tag::Link {
                link_type: LinkType::Autolink | LinkType::Email,
                ..
            }
            | Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript => {
                self.inline(range.start);
                Kind::Inline
            }
            Tag::Link { .. } | Tag::Image { .. } => {
                let counted = matches!(tag, Tag::Link { .. });
                let written = &self.content[range.clone()];
                self.add_text(written, range, Written::Plain);
                self.skipping = Some(Skipping { open: 0, counted });
                return self.fits(0);
            }
            Tag::Paragraph => {
                self.block();
                Kind::Paragraph
            }
            Tag::BlockQuote(_) => {
                self.block();
                Kind::Quote
            }
            Tag::Item => Kind::Item,
            Tag::TableCell => Kind::Cell,
            Tag::Heading { .. }
            | Tag::List(_)
            | Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Table(_)
            | Tag::TableHead
            | Tag::TableRow => {
                self.block();
                Kind::Other
            }
        };
        if !self.fits(0) {
            return false;
        }
        self.stack.push(Parent::new(kind, range.start));
        true
    }
}

/// Finds the spoilers under `parent`, each parent's among its own children.
fn find(parent: &mut Parent, content: &str, found: &mut Vec<Spoiler>) {
    for child in &mut parent.children {
        if let Child::Parent(quote) = child
            && quote.kind == Kind::Quote
        {
            restore_marker(quote, content);
        }
    }
    spoilers_among(&parent.children, content, found);
    for child in &mut parent.children {
        if let Child::Parent(parent) = child {
            find(parent, content, found);
        }
    }
}

/// Gives the text of a quote that is a `>!` spoiler its `>` back: one whose first paragraph
/// starts with text starting `!` and has `!<` in its text, each written as itself.
fn restore_marker(quote: &mut Parent, content: &str) {
    let marker = content[quote.start..]
        .find('>')
        .map_or(quote.start, |at| quote.start + at);
    let Some(Child::Parent(paragraph)) = quote.children.first_mut() else {
        return;
    };
    if paragraph.kind != Kind::Paragraph {
        return;
    }
    let closes = paragraph
        .children
        .iter()
        .any(|child| matches!(child, Child::Text(text) if text.markers.contains("!<")));
    if let Some(Child::Text(first)) = paragraph.children.first_mut()
        && closes
        && first.markers.starts_with('!')
    {
        first.prefix(">", marker..marker + 1);
    }
}

/// A spoiler among one parent's children.
struct Found {
    /// Where its opening marker starts in the text it was looked for in.
    open: usize,
    marker: &'static Marker,
    /// The index of the text its closing marker is in, and where that starts there.
    end: usize,
    close: usize,
}

/// Finds the spoilers whose markers are in the text among `children`, from left to right.
fn spoilers_among(children: &[Child], content: &str, found: &mut Vec<Spoiler>) {
    // For each marker, the first text at or after each child that holds its closing marker,
    // so a run of text is not searched once for every text before it.
    let closers = MARKERS.each_ref().map(|marker| {
        let mut next = None;
        let mut closers = vec![None; children.len() + 1];
        for (index, child) in children.iter().enumerate().rev() {
            if let Child::Text(text) = child
                && let Some(close) = text.markers.find(marker.close)
            {
                next = Some((index, close));
            }
            closers[index] = next;
        }
        closers
    });
    // The text looked through is `children[index]` from `from` on: after a spoiler, what is
    // left of the text its closing marker was in.
    let (mut index, mut from) = (0, 0);
    while let Some(child) = children.get(index) {
        let spoiler = match child {
            Child::Text(text) => first_spoiler(text, from, &closers, index),
            _ => None,
        };
        let (Child::Text(text), Some(spoiler)) = (child, spoiler) else {
            index += 1;
            from = 0;
            continue;
        };
        let Child::Text(last) = &children[spoiler.end] else {
            unreachable!("a closing marker is found in text");
        };
        let closed = spoiler.close + spoiler.marker.close.len();
        found.push(Spoiler {
            whole: text.source_start(spoiler.open, content)..last.source_end(closed - 1),
            inner: text.source_end(spoiler.open + spoiler.marker.open.len() - 1)
                ..last.source_start(spoiler.close, content),
        });
        if closed < last.value.len() {
            index = spoiler.end;
            from = closed;
        } else {
            index = spoiler.end + 1;
            from = 0;
        }
    }
}

/// The first place `marker` opens something in `text` from `from` on: where it opens and
/// where it closes, or `None` for a close when nothing in the text closes it, or `None` when
/// it opens nothing there.
///
/// Openers are tried from left to right, each closing at the nearest closer after it:
///
/// - Something between the two that is not blank is a spoiler.
/// - Only blank text between them (`|| ||`) is none, and both markers are text: the search
///   goes on after the closer.
/// - Nothing between them (`||||`) is none, and the first marker is text: the second is tried
///   as an opener.
/// - No closer after it leaves it open, as it leaves every later opener of the text, so that
///   is the answer: it may close in a later text of the same parent.
///
/// Every step ends at least a marker's length further on, so a text is read once.
fn opening(text: &Text, mut from: usize, marker: &Marker) -> Option<(usize, Option<usize>)> {
    loop {
        let open = from + text.markers[from..].find(marker.open)?;
        let inside = open + marker.open.len();
        let Some(close) = text.markers[inside..].find(marker.close) else {
            return Some((open, None));
        };
        let close = inside + close;
        if close == inside {
            from = inside;
        } else if blank(&text.value[inside..close]) {
            from = close + marker.close.len();
        } else {
            return Some((open, Some(close)));
        }
    }
}

/// The spoiler `text`, the child at `index`, opens from `from` on: each marker's first
/// opening ([`opening`]), closed in that text or else in the next text that has its closing
/// marker, whatever stands between. Of the two markers the one that opens first wins.
fn first_spoiler(
    text: &Text,
    from: usize,
    closers: &[Vec<Option<(usize, usize)>>; 2],
    index: usize,
) -> Option<Found> {
    let mut best: Option<Found> = None;
    for (marker, closers) in MARKERS.iter().zip(closers) {
        let Some((open, close)) = opening(text, from, marker) else {
            continue;
        };
        if best.as_ref().is_some_and(|best| open >= best.open) {
            continue;
        }
        let closed = match close {
            Some(close) => Some((index, close)),
            None => closers[index + 1],
        };
        if let Some((end, close)) = closed {
            best = Some(Found {
                open,
                marker,
                end,
                close,
            });
        }
    }
    best
}

/// Whether `text` is nothing but what JavaScript's `trim` takes off, which is what the client
/// asks of a spoiler's content.
fn blank(text: &str) -> bool {
    text.chars().all(|c| {
        matches!(
            c,
            '\u{9}'..='\u{d}'
                | ' '
                | '\u{a0}'
                | '\u{1680}'
                | '\u{2000}'..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
        )
    })
}

/// Whether a line of `content` opens more than [`MAX_NESTING`] quotes and lists at once, as
/// the client's `opensTooDeeply` counts them: such a message it never parses, and shows as it
/// was written.
fn opens_too_deeply(content: &str) -> bool {
    content.split('\n').any(|line| {
        let bytes = line.as_bytes();
        let space = |at: usize| matches!(bytes.get(at), Some(b' ' | b'\t'));
        // After a bullet or a number: a space, a line's end, or the text's.
        let ends = |at: usize| matches!(bytes.get(at), None | Some(b' ' | b'\t' | b'\r'));
        let mut at = 0;
        let mut markers = 0;
        loop {
            let mut next = at;
            while next < at + 3 && space(next) {
                next += 1;
            }
            next = match bytes.get(next) {
                Some(b'>') => next + 1,
                Some(b'-' | b'*' | b'+') if ends(next + 1) => next + 1,
                Some(b'0'..=b'9') => {
                    let digits = bytes[next..]
                        .iter()
                        .take(9)
                        .take_while(|byte| byte.is_ascii_digit())
                        .count();
                    let after = next + digits;
                    if matches!(bytes.get(after), Some(b'.' | b')')) && ends(after + 1) {
                        after + 1
                    } else {
                        return false;
                    }
                }
                _ => return false,
            };
            if space(next) {
                next += 1;
            }
            markers += 1;
            if markers > MAX_NESTING {
                return true;
            }
            at = next;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    /// What stands for a spoiler in a vector's `shown`, and for a code span or block in
    /// `shown` and `hidden`. Anything else that is not text reads as a space, and whatever is
    /// not in a line of text (a paragraph, a table cell) ends with a line break.
    const SPOILER: char = '▮';
    const CODE: char = '⌷';

    #[derive(Deserialize)]
    struct Vectors {
        vectors: Vec<Vector>,
    }

    #[derive(Deserialize)]
    struct Vector {
        name: String,
        content: String,
        shown: String,
        hidden: Vec<String>,
    }

    /// What a tree reads as, for comparing with a vector.
    struct Read<'a> {
        code: &'a [Range<usize>],
        /// The outermost spoilers, in order.
        spoilers: Vec<Range<usize>>,
        /// Every spoiler's markers.
        markers: Vec<Range<usize>>,
        shown: String,
        hidden: Vec<String>,
        /// How many spoilers `shown` has marked.
        marked: usize,
    }

    impl Read<'_> {
        /// Adds `text`, written at `at` in the source: to `shown`, to what the spoiler it is
        /// in hides, or, a marker, to neither.
        fn add(&mut self, text: &str, at: usize) {
            let inside = self.spoilers.partition_point(|spoiler| spoiler.end <= at);
            if self
                .spoilers
                .get(inside)
                .is_none_or(|spoiler| at < spoiler.start)
            {
                self.shown.push_str(text);
                return;
            }
            if self.marked <= inside {
                self.marked = inside + 1;
                self.shown.push(SPOILER);
            }
            if !self.markers.iter().any(|marker| marker.contains(&at)) {
                self.hidden[inside].push_str(text);
            }
        }

        fn walk(&mut self, parent: &Parent) {
            for child in &parent.children {
                match child {
                    Child::Text(text) => {
                        for (at, c) in text.value.char_indices() {
                            let piece = &text.pieces[text.piece(at)];
                            let source = if piece.verbatim {
                                piece.source.start + (at - piece.at)
                            } else {
                                piece.source.start
                            };
                            self.add(c.encode_utf8(&mut [0; 4]), source);
                        }
                    }
                    Child::Leaf(source) => {
                        let stands = if self.code.contains(source) {
                            CODE
                        } else {
                            ' '
                        };
                        self.add(stands.encode_utf8(&mut [0; 4]), source.start);
                    }
                    Child::Parent(parent) => {
                        self.walk(parent);
                        if parent.kind != Kind::Inline {
                            self.shown.push('\n');
                        }
                    }
                }
            }
        }
    }

    fn spaced(text: &str) -> String {
        text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Every vector reads here as it says the client reads it.
    #[test]
    fn spoilers_are_where_the_vectors_say() {
        let vectors: Vectors =
            serde_json::from_str(include_str!("../../../../../spec/spoiler_vectors.json")).unwrap();
        assert!(!vectors.vectors.is_empty());
        for vector in vectors.vectors {
            let mut tree = Tree::read(&vector.content);
            // In the order written, as the client's tree is.
            tree.root.children.sort_by_key(|child| match child {
                Child::Text(text) => text.pieces[0].source.start,
                Child::Parent(parent) => parent.start,
                Child::Leaf(source) => source.start,
            });
            let markers = tree
                .spoilers
                .iter()
                .flat_map(|spoiler| {
                    [
                        spoiler.whole.start..spoiler.inner.start,
                        spoiler.inner.end..spoiler.whole.end,
                    ]
                })
                .collect();
            let spoilers = outermost(tree.spoilers);
            let mut read = Read {
                code: &tree.code,
                hidden: vec![String::new(); spoilers.len()],
                spoilers,
                markers,
                shown: String::new(),
                marked: 0,
            };
            read.walk(&tree.root);
            assert_eq!(spaced(&read.shown), vector.shown, "{}: shown", vector.name);
            let hidden: Vec<String> = read.hidden.iter().map(|text| spaced(text)).collect();
            assert_eq!(hidden, vector.hidden, "{}: hidden", vector.name);
        }
    }

    fn spoilers(content: &str) -> Vec<&str> {
        read(content)
            .spoilers
            .into_iter()
            .map(|range| &content[range])
            .collect()
    }

    /// A spoiler's range takes its markers as they were written: a table cell's pipes with
    /// their backslashes, and a quote's `>` with what stands between it and the `!`. What it
    /// holds is taken as written too, a character reference whole.
    #[test]
    fn a_spoiler_is_placed_in_the_source() {
        assert_eq!(spoilers("a ||b|| c ||d||"), ["||b||", "||d||"]);
        assert_eq!(
            spoilers("| a |\n|---|\n| x \\|\\|b\\|\\| c |"),
            [r"\|\|b\|\|"]
        );
        assert_eq!(spoilers(r"\\||b||"), ["||b||"]);
        assert_eq!(
            spoilers("a ||&amp;|| &amp; ||b&#124;|| c"),
            ["||&amp;||", "||b&#124;||"]
        );
        assert_eq!(spoilers("x\n\n  > !a\n  > b!< c"), ["> !a\n  > b!<"]);
        assert_eq!(spoilers("||a **||b||** c|| d"), ["||a **||b||** c||"]);
    }

    #[test]
    fn code_is_placed_in_the_source() {
        let content = "`a` b\n\n```\nc\n```\n\n- d\n  ```\n  e\n  ```";
        let code: Vec<&str> = read(content)
            .code
            .into_iter()
            .map(|range| &content[range])
            .collect();
        assert_eq!(code, ["`a`", "```\nc\n```", "```\n  e\n  ```"]);
    }

    /// The count of quote and list markers a line opens with is the client's.
    #[test]
    fn lines_that_open_too_deeply_are_told() {
        assert!(!opens_too_deeply(&">".repeat(MAX_NESTING)));
        assert!(opens_too_deeply(&">".repeat(MAX_NESTING + 1)));
        assert!(opens_too_deeply(&format!(
            "a\n{}x",
            "> - 1. ".repeat(MAX_NESTING / 3 + 1)
        )));
        assert!(!opens_too_deeply(&"-".repeat(MAX_NESTING + 1)));
        assert!(!opens_too_deeply(&"1234567890. ".repeat(MAX_NESTING + 1)));
        assert!(opens_too_deeply(&"   >\t".repeat(MAX_NESTING + 1)));
        assert!(!opens_too_deeply(&"    > ".repeat(MAX_NESTING + 1)));
    }

    /// A marker written with a backslash or as a character reference is none, but for a
    /// table cell's pipe.
    #[test]
    fn a_marker_not_written_as_itself_is_text() {
        assert_eq!(spoilers(r"\|\|a\|\| ||b||"), ["||b||"]);
        assert_eq!(spoilers("&#124;|a|| &gt;!b!&lt; >!c!<"), [">!c!<"]);
        assert_eq!(
            spoilers(r"\>!a!< >\!b!< >!c\!< >!d!\< >!e!<"),
            [r">!c\!< >!d!\< >!e!<"]
        );
        assert_eq!(
            spoilers("| a |\n|---|\n| &#124;&#124;b&#124;&#124; \\>\\!c\\!\\< |"),
            [""; 0]
        );
    }

    /// After markers that hold nothing or only blank text the search goes on, and a long run
    /// of them is read once: were each step to start over, this would not end in a test's time.
    #[test]
    fn markers_that_hold_nothing_are_passed_by() {
        assert_eq!(spoilers("||||a|| || || ||b||"), ["||a||", "||b||"]);
        assert_eq!(spoilers(">!!<>!a!< >! !< ||b||"), [">!a!<", "||b||"]);
        for run in [
            "|".repeat(400_001),
            "|| ".repeat(200_000),
            ">!!<".repeat(100_000),
        ] {
            assert_eq!(spoilers(&run), [""; 0]);
            let closed = format!("{run}||a||");
            assert_eq!(spoilers(&closed).len(), 1, "{}", &closed[..8]);
        }
    }

    /// Text nested deeper than the client draws is read as it shows it: with no Markdown, so
    /// a marker in what would have been code counts.
    #[test]
    fn text_nested_too_deeply_is_read_flat() {
        // A paragraph, emphasis in emphasis, and the code in it: one level more than each
        // `*a _a ` opens two of.
        let nested = |pairs: usize| {
            format!(
                "{}`||a||`{}",
                "*a _a ".repeat(pairs),
                " a_ a*".repeat(pairs)
            )
        };
        let deep = read(&nested(MAX_NESTING / 2));
        assert!(deep.code.is_empty());
        assert_eq!(deep.spoilers.len(), 1);
        let drawn = read(&nested(MAX_NESTING / 2 - 1));
        assert_eq!(drawn.code.len(), 1);
        assert!(drawn.spoilers.is_empty());
        // Lines that open too many quotes are never parsed, and hide nothing.
        let quoted = format!("{}||a||", ">".repeat(MAX_NESTING + 1));
        assert_eq!(read(&quoted), Reading::default());
    }
}
