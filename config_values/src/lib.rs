//! Settings that are lists or tables, read from a configuration file or from the environment.
//!
//! An environment variable holds text, so a setting that is a list or a table cannot be given
//! there as itself. Each such setting deserializes with one of these, which take it as itself
//! from a file, and from text as well:
//!
//! - [`list`]: values separated by commas (`10.0.0.0/8,192.168.0.0/16`), or a JSON array for
//!   values that hold commas themselves. Empty text is an empty list.
//! - [`table`]: a JSON object (`{"POST /users": {"ip": false}}`), for tables whose keys are not
//!   names an environment variable can spell, such as rate limits by endpoint.
//!
//! A table whose keys are plain names (`[rate_limits.default]`) can also be given a key at a
//! time, as `ASPEN_RATE_LIMITS__DEFAULT__IP__REQUESTS`, which the configuration's own nesting
//! does; [`table`] adds the whole-table form beside it.

use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;

/// A list setting, given as a list or as text (see the crate's documentation).
pub fn list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + FromStr,
    T::Err: fmt::Display,
{
    deserializer.deserialize_any(ListVisitor(PhantomData))
}

/// [`list`] for a setting whose absence means something an empty list does not.
pub fn optional_list<'de, D, T>(deserializer: D) -> Result<Option<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + FromStr,
    T::Err: fmt::Display,
{
    list(deserializer).map(Some)
}

/// A table setting, given as a table or as text holding a JSON object.
pub fn table<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    deserializer.deserialize_any(TableVisitor(PhantomData))
}

struct ListVisitor<T>(PhantomData<T>);

impl<'de, T> Visitor<'de> for ListVisitor<T>
where
    T: DeserializeOwned + FromStr,
    T::Err: fmt::Display,
{
    type Value = Vec<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a list, or text of values separated by commas, or a JSON array")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Vec<T>, A::Error> {
        Vec::deserialize(SeqAccessDeserializer::new(seq))
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Vec<T>, E> {
        let text = text.trim();
        if text.starts_with('[') {
            return serde_json::from_str(text)
                .map_err(|error| E::custom(format!("is not a JSON array of the values: {error}")));
        }
        text.split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                value
                    .parse()
                    .map_err(|error| E::custom(format!("{value:?}: {error}")))
            })
            .collect()
    }
}

struct TableVisitor<T>(PhantomData<T>);

impl<'de, T: DeserializeOwned> Visitor<'de> for TableVisitor<T> {
    type Value = T;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a table, or text holding a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
        T::deserialize(MapAccessDeserializer::new(map))
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<T, E> {
        serde_json::from_str(text.trim())
            .map_err(|error| E::custom(format!("is not a JSON object of the table: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::net::IpAddr;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Limit {
        requests: u32,
    }

    #[derive(Debug, Deserialize)]
    struct Settings {
        #[serde(default, deserialize_with = "list")]
        proxies: Vec<String>,
        #[serde(default, deserialize_with = "optional_list")]
        addresses: Option<Vec<IpAddr>>,
        #[serde(default, deserialize_with = "table")]
        endpoints: HashMap<String, Limit>,
    }

    fn from(file: &str, environment: &[(&str, &str)]) -> Result<Settings, config::ConfigError> {
        let variables = environment
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect::<HashMap<_, _>>();
        config::Config::builder()
            .add_source(config::File::from_str(file, config::FileFormat::Toml))
            .add_source(
                config::Environment::with_prefix("ASPEN")
                    .prefix_separator("_")
                    .separator("__")
                    .source(Some(variables)),
            )
            .build()?
            .try_deserialize()
    }

    #[test]
    fn a_file_gives_lists_and_tables_as_themselves() {
        let settings = from(
            "proxies = [\"10.0.0.0/8\"]\naddresses = [\"::1\"]\n[endpoints.\"POST /users\"]\nrequests = 5\n",
            &[],
        )
        .unwrap();
        assert_eq!(settings.proxies, ["10.0.0.0/8"]);
        assert_eq!(settings.addresses, Some(vec!["::1".parse().unwrap()]));
        assert_eq!(settings.endpoints["POST /users"], Limit { requests: 5 });
    }

    #[test]
    fn the_environment_gives_them_as_text() {
        let settings = from(
            "proxies = [\"10.0.0.0/8\"]\n",
            &[
                ("ASPEN_PROXIES", " 127.0.0.1 , 10.0.0.0/8,"),
                ("ASPEN_ADDRESSES", "[\"192.0.2.1\", \"2001:db8::1\"]"),
                ("ASPEN_ENDPOINTS", "{\"POST /users\": {\"requests\": 7}}"),
            ],
        )
        .unwrap();
        assert_eq!(settings.proxies, ["127.0.0.1", "10.0.0.0/8"]);
        assert_eq!(settings.addresses.unwrap().len(), 2);
        assert_eq!(settings.endpoints["POST /users"], Limit { requests: 7 });
    }

    #[test]
    fn empty_text_is_an_empty_list() {
        let settings = from("proxies = [\"10.0.0.0/8\"]\n", &[("ASPEN_PROXIES", "")]).unwrap();
        assert!(settings.proxies.is_empty());
    }

    #[test]
    fn a_wrong_value_says_which() {
        let error = from("", &[("ASPEN_ADDRESSES", "192.0.2.1,not-an-address")]).unwrap_err();
        assert!(error.to_string().contains("not-an-address"), "{error}");
        let error = from("", &[("ASPEN_ENDPOINTS", "{\"POST /users\": 5}")]).unwrap_err();
        assert!(error.to_string().contains("JSON object"), "{error}");
    }
}
