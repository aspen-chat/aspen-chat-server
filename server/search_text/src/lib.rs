//! The text a message is searched by. PostgreSQL's full-text search takes a run of letters for
//! one word, so in a language written without spaces between words (Chinese, Japanese, Thai,
//! Khmer, Lao, Burmese) a whole sentence is one word, and no word of it can be found. This crate
//! finds the words with ICU4X's word segmenter (UAX #29, with its dictionaries and models for
//! those languages) and writes a space between every two that have nothing between them, which
//! PostgreSQL then reads as the words they are. A search's text goes through the same function,
//! so both sides agree on where the words are.
//!
//! The API server computes it for every message as it is written, and `aspen-migrate` for the
//! messages written before the column existed, so both must use this crate at one version:
//! the segmenter's dictionaries decide where a word ends, and a message segmented by one
//! version and searched by another could miss.

use icu_segmenter::options::WordBreakInvariantOptions;
use icu_segmenter::{WordSegmenter, WordSegmenterBorrowed};
use std::sync::LazyLock;

/// One segmenter for the process, built once: its data is compiled in, and it is shared
/// between threads.
static SEGMENTER: LazyLock<WordSegmenterBorrowed<'static>> =
    LazyLock::new(|| WordSegmenter::new_auto(WordBreakInvariantOptions::default()));

/// `content` with a space between every two words that had nothing between them, or `None`
/// when there were no two such words, so that the content is searched as written: nearly
/// every message in a script that writes spaces, which then costs nothing to keep.
///
/// Only spaces are added, between two segments the segmenter calls word-like (letters,
/// including ideographs, and numbers), so everything else, punctuation included, stays as
/// written: the operators of `websearch_to_tsquery` (`"a phrase"`, `-word`, `or`) survive the
/// same pass over a search's text.
pub fn search_text(content: &str) -> Option<String> {
    let mut out = String::new();
    // How much of `content` is in `out`, and where the segment before the current one began.
    let mut copied_to = 0;
    let mut segment_start = 0;
    let mut last_word_like = false;
    for (end, word_type) in SEGMENTER.segment_str(content).iter_with_word_type() {
        let word_like = end > segment_start && word_type.is_word_like();
        if word_like && last_word_like {
            out.push_str(&content[copied_to..segment_start]);
            out.push(' ');
            copied_to = segment_start;
        }
        segment_start = end;
        last_word_like = word_like;
    }
    if copied_to == 0 {
        return None;
    }
    out.push_str(&content[copied_to..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::search_text;

    /// The words of `content` as the search sees them, with what was added taken out again to
    /// check that nothing but spaces was.
    fn words(content: &str) -> Vec<&str> {
        let text = search_text(content).unwrap();
        let leaked: &'static str = Box::leak(text.into_boxed_str());
        assert_eq!(leaked.replace(' ', ""), content.replace(' ', ""));
        leaked.split(' ').collect()
    }

    #[test]
    fn text_written_with_spaces_is_searched_as_written() {
        for content in [
            "apples and pears",
            "don't stop-me now, it's 3.14 or 1,000!",
            "https://example.org/a?b=c#d",
            "<:0191abcd-0000-7000-8000-000000000000>",
            "",
            "한국어는 띄어쓰기를 합니다",
            "🙂🙂",
        ] {
            assert_eq!(search_text(content), None, "{content:?}");
        }
    }

    #[test]
    fn each_word_of_a_language_written_without_spaces_is_one() {
        assert!(words("我喜欢猫和狗").contains(&"猫"));
        assert!(words("こんにちは世界").contains(&"世界"));
        assert!(words("ฉันรักแมว").contains(&"แมว"));
    }

    #[test]
    fn letters_and_ideographs_beside_each_other_are_two_words() {
        assert_eq!(words("iPhone買った")[0], "iPhone");
    }

    #[test]
    fn the_operators_of_a_search_survive() {
        let text = search_text("\"喜欢猫\" -狗 or 鸟").unwrap();
        assert!(text.starts_with("\"喜欢"), "{text}");
        assert!(text.contains("猫\" -狗 or 鸟"), "{text}");
    }
}
