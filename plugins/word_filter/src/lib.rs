//! An example Aspen plugin, and the one the server's tests install: a word filter.
//!
//! - It intercepts messages and, for each word the deployment or the community lists, masks it
//!   with asterisks or refuses the message, as the deployment's `action` setting says. A refusal
//!   in a community whose settings name an `alertChannel` is reported there by its principal.
//! - It observes new messages, annotates those holding one of the community's `watchWords`,
//!   warns of someone posting more than five messages in ten seconds, and counts each channel's
//!   messages in storage scoped to the channel.
//! - Its principal answers the command `/wordcount` with the channel's count, and its route
//!   `GET channels/{channel}/count` answers the same to anyone who may view the channel, for any
//!   channel but one of a plugin's kind, which holds no messages.
//!
//! `aspen-plugin.json` beside this crate is its manifest.

wit_bindgen::generate!({
    path: "../../spec/plugin.wit",
    world: "plugin",
});

use aspen::plugin::host;
use aspen::plugin::types::{
    Annotation, Audience, Context, Draft, Message, Observed, Request, Response, Scope, Severity,
    Text, Verdict,
};
use exports::aspen::plugin::hooks::Guest;
use serde::{Deserialize, Serialize};

struct WordFilter;

/// The deployment's settings.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    words: Vec<String>,
    action: Action,
}

/// What becomes of a message holding a listed word: the manifest's choices for `action`, which a
/// test holds this to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum Action {
    Refuse,
    /// The manifest's default, and what a choice this build does not know is read as, so the
    /// words are still filtered.
    #[default]
    #[serde(other)]
    Mask,
}

/// A community's settings.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct CommunitySettings {
    words: Vec<String>,
    watch_words: Vec<String>,
    alert_channel: Option<String>,
}

fn settings() -> Settings {
    serde_json::from_str(&host::settings()).unwrap_or_default()
}

fn community_settings(context: &Context) -> CommunitySettings {
    context
        .community_settings
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default()
}

fn text(key: &str, args: &[(&str, &str)]) -> Text {
    Text {
        key: key.to_string(),
        args: args
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
    }
}

/// The first of `words` that `content` holds as a whole word, ignoring case.
fn find_word<'a>(content: &str, words: &'a [String]) -> Option<&'a str> {
    let lowered = content.to_lowercase();
    let found: Vec<&str> = lowered
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    words
        .iter()
        .find(|word| found.contains(&word.to_lowercase().as_str()))
        .map(String::as_str)
}

/// `content` with every whole word in `words` replaced by as many asterisks as it has letters.
fn mask(content: &str, words: &[String]) -> String {
    let mut out = String::with_capacity(content.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        let listed = words
            .iter()
            .any(|w| w.to_lowercase() == word.to_lowercase());
        if listed {
            out.extend(std::iter::repeat_n('*', word.chars().count()));
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in content.chars() {
        if c.is_alphanumeric() {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

fn count_key() -> String {
    "messages".to_string()
}

fn channel_count(channel: &str) -> u64 {
    match host::storage_get(&Scope::Channel(channel.to_string()), &count_key()) {
        Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).parse().unwrap_or(0),
        _ => 0,
    }
}

fn observe_message(message: Message, context: &Context) -> Result<(), String> {
    let channel = message.place.channel.clone();
    let count = channel_count(&channel) + 1;
    host::storage_set(
        &Scope::Channel(channel.clone()),
        &count_key(),
        count.to_string().as_bytes(),
    )
    .map_err(|e| format!("{e:?}"))?;
    let community = community_settings(context);
    if let Some(word) = find_word(&message.content, &community.watch_words) {
        host::annotate_message(
            &message.id,
            &Annotation {
                kind: "watched".into(),
                severity: Severity::Notice,
                label: text("watchedLabel", &[("word", word)]),
                detail: Some(text("watchedDetail", &[])),
                link: None,
            },
        )
        .map_err(|e| format!("{e:?}"))?;
    }
    let burst = host::counter_add(&format!("burst:{}", message.author.id), 10, 1)
        .map_err(|e| format!("{e:?}"))?;
    if burst > 5 {
        host::annotate_message(
            &message.id,
            &Annotation {
                kind: "burst".into(),
                severity: Severity::Warning,
                label: text("burstLabel", &[]),
                detail: None,
                link: None,
            },
        )
        .map_err(|e| format!("{e:?}"))?;
    }
    host::publish(
        &Audience::Channel(channel),
        "counted",
        &format!("{{\"count\":{count}}}"),
    )
    .map_err(|e| format!("{e:?}"))?;
    Ok(())
}

impl Guest for WordFilter {
    fn intercept(draft: Draft, context: Context) -> Verdict {
        let settings = settings();
        let community = community_settings(&context);
        let words: Vec<String> = settings.words.into_iter().chain(community.words).collect();
        let Some(word) = find_word(&draft.content, &words) else {
            return Verdict::Allow;
        };
        match settings.action {
            Action::Mask => Verdict::Rewrite(mask(&draft.content, &words)),
            Action::Refuse => {
                if let Some(alert) = community.alert_channel {
                    // Run once this hook has answered.
                    let _ = host::send_message(
                        &alert,
                        &format!(
                            "Refused a message from @{} for \"{word}\".",
                            draft.author.username
                        ),
                    );
                }
                Verdict::Refuse(text("refused", &[("word", word)]))
            }
        }
    }

    fn observe(event: Observed, context: Context) -> Result<(), String> {
        match event {
            Observed::MessageCreated(message) => observe_message(message, &context),
            Observed::CommandInvoked(command) if command.name == "wordcount" => {
                let count = channel_count(&command.place.channel);
                host::send_message(
                    &command.place.channel,
                    &format!("{count} messages counted here."),
                )
                .map(|_| ())
                .map_err(|e| format!("{e:?}"))
            }
            _ => Ok(()),
        }
    }

    fn route(request: Request, _context: Context) -> Response {
        let parts: Vec<&str> = request.path.split('/').collect();
        match (request.method.as_str(), parts.as_slice()) {
            ("GET", ["channels", channel, "count"]) => {
                // A channel of a plugin's kind holds no messages to count. Read as the caller,
                // someone who may not view the channel finds nothing.
                if host::kind_of(channel).map_or(true, |kind| kind.ty == "plugin") {
                    return json(404, "{\"error\":\"notFound\"}".into());
                }
                match host::storage_get(&Scope::Channel(channel.to_string()), &count_key()) {
                    Ok(value) => {
                        let count: u64 = value
                            .map(|bytes| String::from_utf8_lossy(&bytes).parse().unwrap_or(0))
                            .unwrap_or(0);
                        json(200, format!("{{\"count\":{count}}}"))
                    }
                    Err(_) => json(404, "{\"error\":\"notFound\"}".into()),
                }
            }
            _ => json(404, "{\"error\":\"notFound\"}".into()),
        }
    }
}

fn json(status: u16, body: String) -> Response {
    Response {
        status,
        content_type: Some("application/json".into()),
        body: body.into_bytes(),
    }
}

export!(WordFilter);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_the_manifest_offers_is_one_the_filter_knows() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../aspen-plugin.json")).unwrap();
        let setting = manifest["settings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|setting| setting["name"] == "action")
            .unwrap();
        let options = setting["options"].as_array().unwrap();
        assert!(!options.is_empty());
        for option in options {
            // A choice the filter lacks would be read as masking, and written back as `mask`.
            let action: Action = serde_json::from_value(option["value"].clone()).unwrap();
            assert_eq!(serde_json::to_value(action).unwrap(), option["value"]);
        }
        let default: Action = serde_json::from_value(setting["default"].clone()).unwrap();
        assert_eq!(default, Action::default());
    }

    #[test]
    fn an_action_is_read_by_name_and_its_absence_masks() {
        let settings: Settings =
            serde_json::from_str(r#"{"words":["darn"],"action":"refuse"}"#).unwrap();
        assert_eq!(settings.action, Action::Refuse);
        let settings: Settings = serde_json::from_str(r#"{"words":["darn"]}"#).unwrap();
        assert_eq!(settings.action, Action::Mask);
        let settings: Settings =
            serde_json::from_str(r#"{"words":["darn"],"action":"flag"}"#).unwrap();
        assert_eq!((settings.words.len(), settings.action), (1, Action::Mask));
    }

    #[test]
    fn masks_whole_words_ignoring_case() {
        let words = vec!["darn".to_string()];
        assert_eq!(mask("Darn it, darning", &words), "**** it, darning");
        assert_eq!(find_word("oh DARN.", &words), Some("darn"));
        assert_eq!(find_word("darning", &words), None);
    }
}
