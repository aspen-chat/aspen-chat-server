//! Cursors of lists read a page at a time.
//!
//! A cursor is the place of the last row a page returned, in the columns the list is ordered
//! by, so the page after it is a keyset comparison against values the cursor itself holds. It
//! names no row to look up, since that row may be gone by the next request (a member leaves, a
//! voter withdraws), and a page found from a missing row would be empty, which reads as the
//! end of the list. To its reader it is opaque text: base64url of JSON, the list's name and
//! then the position (`["voters",[1700000000000000,"0190…"]]`). The name is there because
//! lists share shapes of position (a time and an id, an id alone), and a cursor of one list
//! read as a place in another would answer a page that means nothing: [`decode`] refuses a
//! cursor of any list but the one asked for.
//!
//! Whether another page follows is decided by reading one row more than the page holds
//! ([`cut`]), never by the page's length.

use crate::t;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The lists read by cursor, one for each order a cursor can be a place in. Its serde names
/// are written into cursors clients hold, so a variant is never renamed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::VariantArray)]
#[serde(rename_all = "camelCase")]
pub enum List {
    /// Someone's activity feed, whichever view and filters it is read with
    /// (`activity::read_activity`).
    Activity,
    /// A community's bans (`ban::read_bans`).
    Bans,
    /// Someone's DMs, as they or the dashboard read them (`dm::list_dms`).
    Dms,
    /// The record of file offers, everyone's or one person's (`file_transfer::read_log`).
    FileOffers,
    /// An author's held messages (`message::held::read_held`).
    HeldMessages,
    /// A community's invites (`invite::read_community_invites`).
    Invites,
    /// A community's members matching a search (`community::search_community_members`).
    Members,
    /// The moderation log (`moderation_log::read_moderation_log`).
    ModerationLog,
    /// Who reacted to a message with one emoji (`react::read_reactors`).
    Reactors,
    /// The report cases of one status, which the position names (`report::list_cases`).
    ReportCases,
    /// The messages someone saved (`saved_message::read_saved_messages`).
    SavedMessages,
    /// A community's server mutes (`voice::mutes::read_mutes`).
    VoiceMutes,
    /// Who voted for one answer of a poll (`poll::read_voters`).
    Voters,
}

/// One page of a list, and the cursor of the page after it: `None` when this page is the last.
#[derive(Debug, PartialEq)]
pub struct Paged<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

/// The longest cursor read, in bytes. Every cursor this server writes is far shorter: the
/// longest position is a member's shown name and an id.
const MAX_CURSOR_BYTES: usize = 2048;

/// The cursor of `position` in `list`.
pub fn encode<P: Serialize>(list: List, position: &P) -> String {
    let json = serde_json::to_vec(&(list, position)).expect("a cursor's position serializes");
    URL_SAFE_NO_PAD.encode(json)
}

/// The position a cursor of `list` holds; a validation error for text that is no cursor of
/// that list, a cursor of another list among it, whatever position it holds.
pub fn decode<P: DeserializeOwned>(list: List, cursor: &str) -> crate::Result<P> {
    let invalid = || crate::Error::Validation(t!("cursorInvalid"));
    if cursor.len() > MAX_CURSOR_BYTES {
        return Err(invalid());
    }
    let json = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| invalid())?;
    let (of, position): (List, serde_json::Value) =
        serde_json::from_slice(&json).map_err(|_| invalid())?;
    if of != list {
        return Err(invalid());
    }
    serde_json::from_value(position).map_err(|_| invalid())
}

/// The instant a position holds as microseconds since the epoch, which is how PostgreSQL keeps
/// one, so a row's time survives its cursor exactly; a validation error for a number no clock
/// tells.
pub fn instant(micros: i64) -> crate::Result<DateTime<Utc>> {
    DateTime::from_timestamp_micros(micros)
        .ok_or_else(|| crate::Error::Validation(t!("cursorInvalid")))
}

/// Cuts `rows`, read in the list's order with a limit of `limit + 1`, to a page of `limit`,
/// with the cursor in `list` of its last row (by `position`) when a row past the page was read.
pub fn cut<R, P: Serialize>(
    list: List,
    mut rows: Vec<R>,
    limit: usize,
    position: impl FnOnce(&R) -> P,
) -> Paged<R> {
    let more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = match rows.last() {
        Some(last) if more => Some(encode(list, &position(last))),
        _ => None,
    };
    Paged {
        items: rows,
        next_cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::VariantArray;
    use uuid::Uuid;

    /// The text a cursor is base64url of.
    fn cursor_of(json: &str) -> String {
        URL_SAFE_NO_PAD.encode(json)
    }

    #[test]
    fn a_position_survives_its_cursor() {
        let position = ("żółw 🐢".to_string(), 7_i64);
        let cursor = encode(List::Members, &position);
        assert!(
            cursor
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "{cursor}"
        );
        assert_eq!(
            decode::<(String, i64)>(List::Members, &cursor).unwrap(),
            position
        );
    }

    #[test]
    fn a_cursor_names_its_list_beside_the_position() {
        let id = Uuid::from_u128(1);
        assert_eq!(
            encode(List::Voters, &(7_i64, id)),
            cursor_of(r#"["voters",[7,"00000000-0000-0000-0000-000000000001"]]"#)
        );
        assert_eq!(
            encode(List::HeldMessages, &id),
            cursor_of(r#"["heldMessages","00000000-0000-0000-0000-000000000001"]"#)
        );
    }

    #[test]
    fn a_cursor_of_another_list_is_refused() {
        let id = Uuid::from_u128(1);
        for &written in List::VARIANTS {
            let placed = encode(written, &(7_i64, id));
            let bare = encode(written, &id);
            for &read in List::VARIANTS {
                assert_eq!(
                    decode::<(i64, Uuid)>(read, &placed).is_ok(),
                    read == written,
                    "{written:?} read as {read:?}"
                );
                assert_eq!(
                    decode::<Uuid>(read, &bare).is_ok(),
                    read == written,
                    "{written:?} read as {read:?}"
                );
            }
        }
    }

    #[test]
    fn text_that_is_no_cursor_is_refused() {
        let refused = |cursor: &str| {
            matches!(
                decode::<(String, i64)>(List::Members, cursor),
                Err(crate::Error::Validation(_))
            )
        };
        assert!(refused(""));
        assert!(refused("not a cursor"));
        assert!(refused("0190a5d4-7b1e-7000-8000-000000000001"));
        // Base64 of something that is not JSON, of JSON of another shape, of a position with
        // no list, and of a list's position with a field too many.
        assert!(refused(&cursor_of("garbage")));
        assert!(refused(&cursor_of(r#"{"name":"a"}"#)));
        assert!(refused(&cursor_of(r#"["a",1]"#)));
        assert!(refused(&cursor_of(r#"["members",["a",1,2]]"#)));
        // A list no cursor is of, a name in another case, and something after the position.
        assert!(refused(&cursor_of(r#"["channels",["a",1]]"#)));
        assert!(refused(&cursor_of(r#"["Members",["a",1]]"#)));
        assert!(refused(&cursor_of(r#"["members",["a",1],2]"#)));
        assert!(!refused(&cursor_of(r#"["members",["a",1]]"#)));
        assert!(refused(&encode(
            List::Members,
            &("a".repeat(MAX_CURSOR_BYTES), 1_i64)
        )));
        assert!(matches!(
            instant(i64::MAX),
            Err(crate::Error::Validation(_))
        ));
        assert_eq!(instant(1_500_000).unwrap().timestamp_micros(), 1_500_000);
    }

    #[test]
    fn a_page_says_whether_another_follows() {
        let position = |row: &i32| *row;
        // A row past the limit was read: the page is cut and carries its last row's place.
        let page = cut(List::Bans, vec![1, 2, 3], 2, position);
        assert_eq!(page.items, vec![1, 2]);
        assert_eq!(
            decode::<i32>(List::Bans, &page.next_cursor.unwrap()).unwrap(),
            2
        );
        // A full page with nothing after it is the last, as a short or empty one is.
        assert_eq!(cut(List::Bans, vec![1, 2], 2, position).next_cursor, None);
        assert_eq!(cut(List::Bans, vec![1], 2, position).next_cursor, None);
        assert_eq!(
            cut(List::Bans, Vec::<i32>::new(), 2, position).next_cursor,
            None
        );
    }
}
