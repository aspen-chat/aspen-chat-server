//! What a message's text names by id: a person tagged (`<@id>`), a role tagged (`<@&id>`), and
//! a community's emoji (`<:id>`), each id a UUID in its hyphenated form. Whoever writes a
//! message's text out for someone who cannot resolve ids (an email, a phone's notification)
//! reads the tokens here.

use crate::mention::{self, Found};
use crate::{CustomEmojiId, RoleId, UserId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Token {
    User(UserId),
    Role(RoleId),
    Emoji(CustomEmojiId),
}

/// The length of an emoji's reference in bytes: `<:`, a hyphenated UUID, `>`.
const EMOJI_REFERENCE_LEN: usize = 39;

/// The token at the start of `text`, and its length in bytes. Only the bytes a token can hold
/// are read, so text of many `<` and no token is read once.
fn token_at(text: &str) -> Option<(Token, usize)> {
    if let Some((tag, len)) = mention::tag_at(text) {
        let token = match tag {
            Found::User(id) => Token::User(UserId(id)),
            Found::Role(id) => Token::Role(RoleId(id)),
        };
        return Some((token, len));
    }
    let reference = text.get(..EMOJI_REFERENCE_LEN)?;
    crate::custom_emoji::referenced(reference).map(|id| (Token::Emoji(id), EMOJI_REFERENCE_LEN))
}

/// `text` with each token written as `name` gives it; one it gives nothing for stays as
/// written.
pub(crate) fn rewritten(text: &str, mut name: impl FnMut(Token) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        match token_at(rest).and_then(|(token, len)| Some((name(token)?, len))) {
            Some((named, len)) => {
                out.push_str(&named);
                rest = &rest[len..];
            }
            None => {
                out.push('<');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Gives `each` every token in `text`, in the order written.
pub(crate) fn each(text: &str, mut each: impl FnMut(Token)) {
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        rest = &rest[start..];
        match token_at(rest) {
            Some((token, len)) => {
                each(token);
                rest = &rest[len..];
            }
            None => rest = &rest[1..],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";

    fn id() -> uuid::Uuid {
        ID.parse().unwrap()
    }

    #[test]
    fn each_kind_of_token_is_read() {
        let mut found = Vec::new();
        each(&format!("a <@{ID}> b <@&{ID}> c <:{ID}> d"), |token| {
            found.push(token)
        });
        assert_eq!(
            found,
            vec![
                Token::User(UserId(id())),
                Token::Role(RoleId(id())),
                Token::Emoji(CustomEmojiId(id())),
            ]
        );
    }

    /// What only looks like a token is passed by a character at a time, so one after it is
    /// still found, and text of other scripts is cut nowhere but at its characters.
    #[test]
    fn what_is_no_token_stays_as_written() {
        let text = format!("<:) ж< <@nope> <:{ID} > <@{ID}><: <");
        let named = rewritten(&text, |token| match token {
            Token::User(_) => Some("@Alex".to_string()),
            _ => None,
        });
        assert_eq!(named, format!("<:) ж< <@nope> <:{ID} > @Alex<: <"));
        // The id in its 32-character form is not how a client writes a token.
        let simple = ID.replace('-', "");
        let mut found = 0;
        each(&format!("<@{simple}> <:{simple}>"), |_| found += 1);
        assert_eq!(found, 0);
    }

    #[test]
    fn a_token_nothing_names_stays_as_written() {
        let text = format!("<:{ID}> and <@&{ID}>");
        assert_eq!(rewritten(&text, |_| None), text);
    }
}
