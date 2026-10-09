//! The words seeded history and live benchmark messages are written in, and searched for.
//!
//! Words are drawn by rank with Zipf's law (the `n`th word about `1/n` as often as the first),
//! as words in real chat are, so a search for a common word matches much of a channel and one
//! for a rare word almost none of it, and a run's searches cover both. The seeder and the tool
//! share this list so that the words searched for are the words written. A few are in scripts
//! other than Latin, so search is exercised beyond it.

use std::sync::LazyLock;

/// Most common first.
pub const WORDS: &[&str] = &[
    "the",
    "i",
    "to",
    "a",
    "you",
    "and",
    "it",
    "is",
    "that",
    "of",
    "in",
    "lol",
    "for",
    "on",
    "we",
    "this",
    "be",
    "just",
    "so",
    "have",
    "but",
    "what",
    "not",
    "are",
    "do",
    "with",
    "was",
    "can",
    "yeah",
    "if",
    "me",
    "my",
    "all",
    "at",
    "like",
    "get",
    "one",
    "they",
    "now",
    "up",
    "go",
    "out",
    "about",
    "think",
    "good",
    "know",
    "game",
    "time",
    "how",
    "when",
    "there",
    "ok",
    "will",
    "or",
    "your",
    "no",
    "here",
    "an",
    "got",
    "see",
    "anyone",
    "tonight",
    "people",
    "would",
    "new",
    "really",
    "back",
    "still",
    "server",
    "play",
    "want",
    "thanks",
    "make",
    "some",
    "day",
    "much",
    "need",
    "today",
    "nice",
    "work",
    "right",
    "way",
    "thing",
    "first",
    "last",
    "never",
    "always",
    "maybe",
    "sure",
    "again",
    "pretty",
    "everyone",
    "map",
    "patch",
    "match",
    "team",
    "round",
    "voice",
    "stream",
    "build",
    "update",
    "event",
    "schedule",
    "link",
    "channel",
    "music",
    "movie",
    "photo",
    "screenshot",
    "weekend",
    "morning",
    "evening",
    "later",
    "soon",
    "ready",
    "hello",
    "welcome",
    "congrats",
    "awesome",
    "great",
    "weird",
    "funny",
    "wild",
    "late",
    "early",
    "lunch",
    "dinner",
    "coffee",
    "snacks",
    "pizza",
    "raid",
    "quest",
    "boss",
    "loot",
    "level",
    "ranked",
    "casual",
    "queue",
    "lag",
    "ping",
    "crash",
    "bug",
    "fix",
    "release",
    "beta",
    "trailer",
    "review",
    "guide",
    "tips",
    "question",
    "answer",
    "idea",
    "plan",
    "vote",
    "poll",
    "thread",
    "announcement",
    "rules",
    "role",
    "invite",
    "community",
    "friends",
    "party",
    "birthday",
    "holiday",
    "vacation",
    "travel",
    "weather",
    "rain",
    "snow",
    "summer",
    "winter",
    "football",
    "basketball",
    "chess",
    "puzzle",
    "art",
    "drawing",
    "painting",
    "garden",
    "cat",
    "dog",
    "recipe",
    "bread",
    "tea",
    "keyboard",
    "monitor",
    "headset",
    "microphone",
    "camera",
    "laptop",
    "phone",
    "battery",
    "charger",
    "router",
    "kernel",
    "compiler",
    "database",
    "rust",
    "python",
    "typescript",
    "deploy",
    "benchmark",
    "latency",
    "throughput",
    "memory",
    "aurora",
    "comet",
    "glacier",
    "harbor",
    "lantern",
    "meadow",
    "nebula",
    "orchard",
    "quartz",
    "saffron",
    "tundra",
    "velvet",
    "willow",
    "zephyr",
    "obsidian",
    "marmalade",
    "kaleidoscope",
    "привет",
    "спасибо",
    "καλημέρα",
    "ευχαριστώ",
    "ありがとう",
    "こんにちは",
    "谢谢",
    "你好",
    "مرحبا",
    "شكرا",
    "नमस्ते",
    "धन्यवाद",
    "안녕하세요",
    "감사합니다",
    "שלום",
];

/// The cumulative share of the words up to each rank.
static CUMULATIVE: LazyLock<Vec<f64>> = LazyLock::new(|| {
    let weights: Vec<f64> = (1..=WORDS.len()).map(|rank| 1.0 / rank as f64).collect();
    let total: f64 = weights.iter().sum();
    let mut running = 0.0;
    weights
        .into_iter()
        .map(|weight| {
            running += weight / total;
            running
        })
        .collect()
});

/// The word a uniform draw `u` in `[0, 1)` falls on.
pub fn word(u: f64) -> &'static str {
    let rank = CUMULATIVE.partition_point(|&share| share <= u);
    WORDS[rank.min(WORDS.len() - 1)]
}

/// A line of three to twelve words, each drawn from `draw`, which yields uniform draws in
/// `[0, 1)`.
pub fn line(mut draw: impl FnMut() -> f64) -> String {
    let count = 3 + (draw() * 10.0) as usize;
    (0..count)
        .map(|_| word(draw()))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_fall_by_rank() {
        assert_eq!(word(0.0), "the");
        assert_eq!(word(0.999_999), WORDS[WORDS.len() - 1]);
        let mut counter = 0u32;
        let mut draw = || {
            counter += 1;
            f64::from(counter % 97) / 97.0
        };
        let text = line(&mut draw);
        let words = text.split(' ').count();
        assert!((3..=12).contains(&words), "{text}");
    }

    #[test]
    fn every_word_is_one_searchable_word() {
        let mut seen = std::collections::HashSet::new();
        for word in WORDS {
            assert!(!word.is_empty() && !word.contains(char::is_whitespace));
            assert!(seen.insert(word), "{word} is listed twice");
        }
    }
}
