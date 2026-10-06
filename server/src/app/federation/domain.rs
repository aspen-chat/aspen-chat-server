//! A deployment's name ([`Domain`]), as written in configuration, URLs, and the directory.

use super::WELL_KNOWN_PATH;
use crate::app;
use crate::aspen_config::FederationConfig;
use diesel::{AsExpression, FromSqlRow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use utoipa::ToSchema;

/// A deployment's name: a DNS name of at least two labels, in lowercase, with `:port` when it
/// is served on a port other than 443. It is never an IP address, since a deployment's
/// certificate and identity belong to a name.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    ToSchema,
    FromSqlRow,
    AsExpression,
)]
#[serde(try_from = "String", into = "String")]
#[schema(value_type = String, example = "chat.example.org")]
#[diesel(sql_type = diesel::sql_types::Text)]
pub struct Domain(String);

#[derive(Debug, thiserror::Error)]
#[error("not a deployment domain")]
pub struct InvalidDomain;

impl Domain {
    /// Reads a domain as someone might write it: surrounding space and case do not matter, and
    /// `:443` is the same as no port.
    pub fn parse(text: &str) -> Result<Self, InvalidDomain> {
        let text = text.trim().to_ascii_lowercase();
        let (host, port) = match text.rsplit_once(':') {
            Some((host, port)) => {
                // Only digits: `u16::from_str` would take a leading `+`.
                if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(InvalidDomain);
                }
                let port: u16 = port.parse().map_err(|_| InvalidDomain)?;
                if port == 0 {
                    return Err(InvalidDomain);
                }
                (host, (port != 443).then_some(port))
            }
            None => (text.as_str(), None),
        };
        let host = host.strip_suffix('.').unwrap_or(host);
        let labels: Vec<&str> = host.split('.').collect();
        let label_ok = |label: &&str| {
            (1..=63).contains(&label.len())
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        };
        if host.len() > 253
            || labels.len() < 2
            || !labels.iter().all(label_ok)
            // A last label of digits alone would make an IPv4 address a domain.
            || labels.last().is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit()))
            // Nor may a URL read it as one (`127.0x1` is 127.0.0.1 there), since a client
            // connects to an address without resolving it, past `app::outbound::PublicResolver`.
            || !matches!(url::Host::parse(host), Ok(url::Host::Domain(_)))
        {
            return Err(InvalidDomain);
        }
        Ok(Domain(match port {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        }))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Where the deployment publishes its document.
    pub fn document_url(&self) -> String {
        format!("https://{}{WELL_KNOWN_PATH}", self.0)
    }
}

impl fmt::Display for Domain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Domain {
    type Err = InvalidDomain;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl TryFrom<String> for Domain {
    type Error = InvalidDomain;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<Domain> for String {
    fn from(domain: Domain) -> Self {
        domain.0
    }
}

app::text_sql_traits!(Domain);

impl JsonSchema for Domain {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Domain".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "A deployment's name: a DNS name of two or more labels, in lowercase, \
                with `:port` when it is not served on 443.",
            "examples": ["chat.example.org"]
        })
    }
}

/// This deployment's own domain, if it has one.
pub fn own_domain(config: &FederationConfig) -> Option<Domain> {
    config.domain.as_deref().and_then(|d| Domain::parse(d).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_are_read_as_people_write_them() {
        let d = |text: &str| Domain::parse(text).map(String::from).ok();
        assert_eq!(d(" Chat.Example.org "), Some("chat.example.org".into()));
        assert_eq!(d("chat.example.org:443"), Some("chat.example.org".into()));
        assert_eq!(
            d("alpha.localhost:8443"),
            Some("alpha.localhost:8443".into())
        );
        assert_eq!(d("chat.example.org."), Some("chat.example.org".into()));
        for bad in [
            "localhost",
            "10.0.0.1",
            "https://chat.example.org",
            "chat.example.org/path",
            "chat..example.org",
            "-chat.example.org",
            "chat.example.org:0",
            "chat.example.org:+80",
            "chat.example.org:99999",
            "chat_example.org",
            "[::1]:443",
            "127.0x1",
            "1.0x1:8443",
            "0x7f.0x0.0x0.0x1",
            "10.0x0a000001",
            "",
        ] {
            assert_eq!(d(bad), None, "{bad}");
        }
    }
}
