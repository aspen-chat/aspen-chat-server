//! Connecting to PostgreSQL as `database_url` says, over TLS when it asks for it.
//!
//! `database_url` is a libpq connection string, either a URL
//! (`postgres://aspen@db.internal/aspen?sslmode=verify-full`) or `key=value` pairs, and takes
//! libpq's TLS parameters with libpq's meanings:
//!
//! - `sslmode`: `disable` never encrypts. `prefer`, the default, encrypts when the server offers
//!   TLS, and `require` refuses a server that does not; neither checks the server's certificate
//!   unless `sslrootcert` names a file, when both check it was issued by an authority in that
//!   file. `verify-ca` always checks that, and needs `sslrootcert`. `verify-full` also checks the
//!   certificate names the host connected to, trusting `sslrootcert`'s authorities or, without
//!   it, the system's. libpq's `allow` (plaintext first, TLS only when the server insists) is
//!   refused, since tokio-postgres cannot try both.
//! - `sslrootcert`: a PEM file of the authorities to trust, alone, or `system` for OpenSSL's
//!   (`SSL_CERT_FILE` and `SSL_CERT_DIR` replace them), which as in libpq makes the default mode
//!   `verify-full` and refuses any weaker one: any public authority vouches for some name.
//! - `sslcert` and `sslkey`: PEM files of a client certificate (its chain, leaf first) and its
//!   key, for a server that authenticates clients by certificate. Both or neither.
//!
//! An unchecked certificate keeps out only those who can read the traffic, not those who can
//! change it. SCRAM binds the sign-in to the certificate the client saw (`tls-server-end-point`),
//! so a server in the middle cannot relay it, but it can ask for the password in the clear
//! instead; `verify-full`, or `channel_binding=require`, which refuses every sign-in but a bound
//! one, closes that. Every other parameter is tokio-postgres's to read.
//!
//! TLS is OpenSSL's, as libpq's is, through tokio-postgres's own `postgres-openssl`, and each
//! mode is the OpenSSL setting libpq makes for it: no verification, verification without the
//! host name, or OpenSSL's full verification. What is this crate's own is reading the TLS
//! parameters out of `database_url`, which its tests hold to tokio-postgres's reading of the rest.

use diesel_async::AsyncPgConnection;
use diesel_async::pooled_connection::{AsyncDieselConnectionManager, ManagerConfig};
use futures_util::FutureExt;
use openssl::ssl::{SslConnector, SslFiletype, SslMethod, SslVerifyMode};
use openssl::x509::store::X509StoreBuilder;
use postgres_openssl::MakeTlsConnector;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_postgres::config::SslMode;

/// Why `database_url` cannot be connected with.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database_url is not a connection string: {0}")]
    Syntax(&'static str),
    #[error("database_url is not a connection string tokio-postgres accepts: {0}")]
    Config(#[from] tokio_postgres::Error),
    #[error("database_url's {param} is {value:?}; it may be {expected}")]
    Value {
        param: &'static str,
        value: String,
        expected: &'static str,
    },
    #[error("database_url's sslmode is {mode}, which sslrootcert=system refuses; use verify-full")]
    WeakerThanSystem { mode: String },
    #[error("database_url's sslmode is verify-ca, which needs sslrootcert to name the authority")]
    VerifyCaWithoutRoot,
    #[error("database_url names {0} without {1}; a client certificate needs both")]
    Unpaired(&'static str, &'static str),
    #[error("could not use {path}, which database_url's {param} names: {source}")]
    File {
        param: &'static str,
        path: PathBuf,
        source: openssl::error::ErrorStack,
    },
    #[error("OpenSSL could not set up TLS for the database: {0}")]
    Tls(#[from] openssl::error::ErrorStack),
}

/// Where a connection's TLS session gets the authorities it trusts.
enum Roots {
    File(PathBuf),
    System,
}

/// How a connection checks the server's certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Check {
    /// Encrypts against whoever answers.
    None,
    /// The certificate was issued by a trusted authority, for any name.
    Authority,
    /// The certificate was issued by a trusted authority for the host connected to.
    Full,
}

/// A parsed `database_url`: what to connect to, and how to secure it.
#[derive(Clone)]
pub struct Database {
    config: tokio_postgres::Config,
    tls: MakeTlsConnector,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // tokio-postgres's `Debug` leaves out the password.
        f.debug_struct("Database")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl std::str::FromStr for Database {
    type Err = Error;

    fn from_str(url: &str) -> Result<Self, Error> {
        let (rest, tls) = take_tls_params(url)?;
        let mut config: tokio_postgres::Config = rest.parse()?;
        let roots = match tls.root_cert.as_deref() {
            None => None,
            Some("system") => Some(Roots::System),
            Some(path) => Some(Roots::File(path.into())),
        };
        let mode = tls.mode.as_deref();
        let (ssl_mode, check) = match (mode, &roots) {
            (Some("allow"), _) => {
                return Err(Error::Value {
                    param: "sslmode",
                    value: "allow".into(),
                    expected: "disable, prefer, require, verify-ca, or verify-full",
                });
            }
            (None | Some("verify-full"), Some(Roots::System)) => (SslMode::Require, Check::Full),
            (
                Some(mode @ ("disable" | "prefer" | "require" | "verify-ca")),
                Some(Roots::System),
            ) => {
                return Err(Error::WeakerThanSystem { mode: mode.into() });
            }
            (Some("disable"), _) => (SslMode::Disable, Check::None),
            (None | Some("prefer"), None) => (SslMode::Prefer, Check::None),
            (None | Some("prefer"), Some(_)) => (SslMode::Prefer, Check::Authority),
            (Some("require"), None) => (SslMode::Require, Check::None),
            (Some("require"), Some(_)) => (SslMode::Require, Check::Authority),
            (Some("verify-ca"), None) => return Err(Error::VerifyCaWithoutRoot),
            (Some("verify-ca"), Some(_)) => (SslMode::Require, Check::Authority),
            (Some("verify-full"), _) => (SslMode::Require, Check::Full),
            (Some(other), _) => {
                return Err(Error::Value {
                    param: "sslmode",
                    value: other.into(),
                    expected: "disable, prefer, require, verify-ca, or verify-full",
                });
            }
        };
        config.ssl_mode(ssl_mode);
        let tls = connector(
            check,
            roots.unwrap_or(Roots::System),
            tls.cert.as_deref().map(Path::new),
            tls.key.as_deref().map(Path::new),
        )?;
        Ok(Self { config, tls })
    }
}

/// The password `url` gives, if any, read without opening the files its TLS parameters name.
pub fn password(url: &str) -> Result<Option<Vec<u8>>, Error> {
    let (rest, _) = take_tls_params(url)?;
    let config: tokio_postgres::Config = rest.parse()?;
    Ok(config.get_password().map(<[u8]>::to_vec))
}

impl Database {
    /// Opens one connection.
    pub async fn connect(&self) -> diesel::ConnectionResult<AsyncPgConnection> {
        let (client, connection) = self
            .config
            .connect(self.tls.clone())
            .await
            .map_err(|e| diesel::ConnectionError::BadConnection(e.to_string()))?;
        AsyncPgConnection::try_from_client_and_connection(client, connection).await
    }

    /// What a pool opens its connections with.
    pub fn manager(self) -> AsyncDieselConnectionManager<AsyncPgConnection> {
        let database = Arc::new(self);
        let mut config = ManagerConfig::default();
        config.custom_setup = Box::new(move |_| {
            let database = database.clone();
            async move { database.connect().await }.boxed()
        });
        // The manager hands its URL only to `custom_setup`, which ignores it.
        AsyncDieselConnectionManager::new_with_config("", config)
    }
}

/// Parses `url` and opens one connection with it.
pub async fn connect(url: &str) -> Result<AsyncPgConnection, ConnectError> {
    Ok(url.parse::<Database>()?.connect().await?)
}

/// Why [`connect`] failed.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error(transparent)]
    Url(#[from] Error),
    #[error(transparent)]
    Connection(#[from] diesel::ConnectionError),
}

/// The libpq TLS parameters tokio-postgres does not read.
#[derive(Debug, Default, PartialEq, Eq)]
struct TlsParams {
    mode: Option<String>,
    root_cert: Option<String>,
    cert: Option<String>,
    key: Option<String>,
}

impl TlsParams {
    /// Takes `key`'s value when it is one of these, answering whether it was.
    fn take(&mut self, key: &str, value: String) -> bool {
        let slot = match key {
            "sslmode" => &mut self.mode,
            "sslrootcert" => &mut self.root_cert,
            "sslcert" => &mut self.cert,
            "sslkey" => &mut self.key,
            _ => return false,
        };
        *slot = Some(value);
        true
    }
}

/// Splits the TLS parameters out of `url`, returning the rest in its own form for tokio-postgres,
/// which refuses parameters it does not know and knows only three of libpq's `sslmode`s. It reads
/// both forms as tokio-postgres does.
fn take_tls_params(url: &str) -> Result<(String, TlsParams), Error> {
    let mut tls = TlsParams::default();
    if let Some(rest) = ["postgres://", "postgresql://"]
        .iter()
        .find_map(|prefix| url.strip_prefix(prefix))
    {
        // Credentials end at the first `@`, and the parameters start at the first `?` after.
        let after_credentials = rest.find('@').map_or(0, |at| at + 1);
        let Some(query_start) = rest[after_credentials..]
            .find('?')
            .map(|q| url.len() - rest.len() + after_credentials + q)
        else {
            return Ok((url.into(), tls));
        };
        let mut kept = Vec::new();
        for pair in url[query_start + 1..].split('&') {
            let decode = |s: &str| {
                percent_encoding::percent_decode_str(s)
                    .decode_utf8()
                    .map(String::from)
                    .map_err(|_| Error::Syntax("a parameter is not UTF-8 once decoded"))
            };
            let Some((key, value)) = pair.split_once('=') else {
                return Err(Error::Syntax("a parameter has no `=`"));
            };
            if !tls.take(&decode(key)?, decode(value)?) {
                kept.push(pair);
            }
        }
        let mut rest = url[..query_start].to_string();
        if !kept.is_empty() {
            rest.push('?');
            rest.push_str(&kept.join("&"));
        }
        return Ok((rest, tls));
    }
    let mut rest = String::new();
    for (key, value) in key_value_pairs(url)? {
        if !tls.take(key, value.clone()) {
            if !rest.is_empty() {
                rest.push(' ');
            }
            rest.push_str(key);
            rest.push_str("='");
            for c in value.chars() {
                if c == '\'' || c == '\\' {
                    rest.push('\\');
                }
                rest.push(c);
            }
            rest.push('\'');
        }
    }
    Ok((rest, tls))
}

/// Reads libpq's `key = value` form: values bare up to whitespace or single-quoted, with `\`
/// escaping the next character in either.
fn key_value_pairs(s: &str) -> Result<Vec<(&str, String)>, Error> {
    let mut pairs = Vec::new();
    let mut rest = s.trim_start();
    while !rest.is_empty() {
        let key_end = rest
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(rest.len());
        let key = &rest[..key_end];
        if key.is_empty() {
            return Err(Error::Syntax("a parameter has no name"));
        }
        rest = rest[key_end..].trim_start();
        rest = rest
            .strip_prefix('=')
            .ok_or(Error::Syntax("a parameter has no `=`"))?
            .trim_start();
        let quoted = rest.starts_with('\'');
        let mut chars = rest.char_indices().skip(usize::from(quoted));
        let mut value = String::new();
        let mut end = None;
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => value.extend(chars.next().map(|(_, c)| c)),
                '\'' if quoted => {
                    end = Some(i + 1);
                    break;
                }
                c if !quoted && c.is_whitespace() => {
                    end = Some(i);
                    break;
                }
                c => value.push(c),
            }
        }
        rest = match end {
            Some(end) => &rest[end..],
            None if quoted => return Err(Error::Syntax("a quoted value is never closed")),
            None => "",
        };
        if !quoted && value.is_empty() {
            return Err(Error::Syntax("a parameter has no value"));
        }
        pairs.push((key, value));
        rest = rest.trim_start();
    }
    Ok(pairs)
}

/// The TLS client a connection uses: checking as `check` says against `roots`, presenting the
/// client certificate when one is given, and offering ALPN's `postgresql`, which PostgreSQL 17
/// requires of a direct TLS connection (`sslnegotiation=direct`) and earlier versions ignore.
fn connector(
    check: Check,
    roots: Roots,
    cert: Option<&Path>,
    key: Option<&Path>,
) -> Result<MakeTlsConnector, Error> {
    // Verifies against OpenSSL's default authorities, and the host name, unless told otherwise.
    let mut builder = SslConnector::builder(SslMethod::tls_client())?;
    if let Roots::File(path) = &roots {
        builder.set_cert_store(X509StoreBuilder::new()?.build());
        builder
            .set_ca_file(path)
            .map_err(file("sslrootcert", path))?;
    }
    if check == Check::None {
        builder.set_verify(SslVerifyMode::NONE);
    }
    match (cert, key) {
        (None, None) => {}
        (Some(cert), Some(key)) => {
            builder
                .set_certificate_chain_file(cert)
                .map_err(file("sslcert", cert))?;
            builder
                .set_private_key_file(key, SslFiletype::PEM)
                .map_err(file("sslkey", key))?;
            builder.check_private_key().map_err(file("sslkey", key))?;
        }
        (Some(_), None) => return Err(Error::Unpaired("sslcert", "sslkey")),
        (None, Some(_)) => return Err(Error::Unpaired("sslkey", "sslcert")),
    }
    postgres_openssl::set_postgresql_alpn(&mut builder)?;
    let mut connector = MakeTlsConnector::new(builder.build());
    if check != Check::Full {
        connector.set_callback(|connection, _| {
            connection.set_verify_hostname(false);
            Ok(())
        });
    }
    Ok(connector)
}

/// What becomes of OpenSSL's refusal of the file at `path`, which `param` names.
fn file<'a>(
    param: &'static str,
    path: &'a Path,
) -> impl FnOnce(openssl::error::ErrorStack) -> Error + 'a {
    move |source| Error::File {
        param,
        path: path.into(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(url: &str) -> (String, TlsParams) {
        take_tls_params(url).unwrap()
    }

    #[test]
    fn takes_tls_parameters_out_of_a_url() {
        let (rest, tls) = split(
            "postgres://u:p%3Fw@db:5432/aspen?sslmode=verify-full&connect_timeout=10\
             &sslrootcert=%2Fetc%2Fca.pem&options=-c%20a%3Db",
        );
        assert_eq!(
            rest,
            "postgres://u:p%3Fw@db:5432/aspen?connect_timeout=10&options=-c%20a%3Db"
        );
        assert_eq!(tls.mode.as_deref(), Some("verify-full"));
        assert_eq!(tls.root_cert.as_deref(), Some("/etc/ca.pem"));
        let (rest, tls) = split("postgresql://u:a?b@db/aspen?sslmode=require");
        assert_eq!(rest, "postgresql://u:a?b@db/aspen");
        assert_eq!(tls.mode.as_deref(), Some("require"));
        let (rest, tls) = split("postgres://db/aspen");
        assert_eq!(rest, "postgres://db/aspen");
        assert_eq!(tls, TlsParams::default());
    }

    #[test]
    fn takes_tls_parameters_out_of_key_value_pairs() {
        let (rest, tls) = split(
            "host=db  user = aspen password='it\\'s a secret' sslmode=verify-ca \
             sslrootcert='/etc/my ca.pem'",
        );
        assert_eq!(rest, "host='db' user='aspen' password='it\\'s a secret'");
        assert_eq!(tls.mode.as_deref(), Some("verify-ca"));
        assert_eq!(tls.root_cert.as_deref(), Some("/etc/my ca.pem"));
        let config: tokio_postgres::Config = rest.parse().unwrap();
        assert_eq!(config.get_password(), Some(&b"it's a secret"[..]));
        assert!(take_tls_params("host='db").is_err());
        assert!(take_tls_params("host").is_err());
    }

    /// Takes `n`'s digits in each of `bases`, so every combination of choices is one number.
    fn choices(mut n: usize, bases: &[usize]) -> Vec<usize> {
        bases
            .iter()
            .map(|base| {
                let choice = n % base;
                n /= base;
                choice
            })
            .collect()
    }

    /// Holds the reading of TLS parameters to tokio-postgres's reading of everything else: for
    /// thousands of connection strings in both forms, with TLS parameters among the others or
    /// none, what is left once they are taken out parses to what the same string written without
    /// them does. Reserved characters in credentials, quoting, escapes, and several hosts are
    /// among them, since a misreading there could drop `sslmode` or hand tokio-postgres a
    /// different password.
    #[test]
    fn leaves_tokio_postgres_what_it_would_read_without_them() {
        let tls_params = [
            ("sslmode", "verify-full", "verify-full"),
            ("sslrootcert", "%2Fetc%2Fmy%20ca.pem", "'/etc/my ca.pem'"),
            ("sslcert", "c.pem", "c.pem"),
            ("sslkey", "k%3D.pem", "'k=.pem'"),
        ];
        let prefixes = ["postgres://", "postgresql://"];
        let credentials = [
            "",
            "u@",
            "u:p@",
            "u:p%3Fw@",
            "u:a?b@",
            "us%40er:p%26w=@",
            "u:p&q@",
        ];
        let hosts = [
            "db",
            "db:5432",
            "[::1]:5432",
            "h1,h2:5433",
            "",
            "%2Fvar%2Frun",
        ];
        let paths = ["", "/aspen", "/as%20pen"];
        let others: [&[&str]; 4] = [
            &[],
            &["connect_timeout=10"],
            &["options=-c%20a%3Db", "application_name=a+b"],
            &["host=%2Ftmp", "user=x"],
        ];
        let bases = [
            prefixes.len(),
            credentials.len(),
            hosts.len(),
            paths.len(),
            others.len(),
            16,
            3,
        ];
        let mut compared = 0;
        for n in 0..bases.iter().product() {
            let c = choices(n, &bases);
            let base = format!(
                "{}{}{}{}",
                prefixes[c[0]], credentials[c[1]], hosts[c[2]], paths[c[3]]
            );
            let kept = others[c[4]];
            let taken: Vec<_> = (0..tls_params.len())
                .filter(|i| c[5] & (1 << i) != 0)
                .map(|i| format!("{}={}", tls_params[i].0, tls_params[i].1))
                .collect();
            // TLS parameters first, last, or between the others.
            let mut params: Vec<String> = kept.iter().map(|p| p.to_string()).collect();
            let at = [0, params.len(), params.len().min(1)][c[6]];
            params.splice(at..at, taken.iter().cloned());
            let join = |params: &[String]| {
                if params.is_empty() {
                    base.clone()
                } else {
                    format!("{base}?{}", params.join("&"))
                }
            };
            let with = join(&params);
            let without = join(&kept.iter().map(|p| p.to_string()).collect::<Vec<_>>());
            let expected = without.parse::<tokio_postgres::Config>().ok();
            let (rest, tls) = match take_tls_params(&with) {
                Ok(split) => split,
                Err(_) => {
                    assert!(expected.is_none(), "{with} was refused");
                    continue;
                }
            };
            assert_eq!(
                rest.parse::<tokio_postgres::Config>().ok(),
                expected,
                "{with}"
            );
            assert_eq!(tls.mode.is_some(), c[5] & 1 != 0, "{with}");
            compared += 1;
        }

        let pairs = [
            &["host=db", "host='db'", "host = '/var/run/postgresql'"][..],
            &["user=aspen", "user='as pen'", "user=as\\ pen"],
            &[
                "password=secret",
                "password='it\\'s'",
                "password='a=b c'",
                "password=p\\\\w",
                "password='sslmode=disable'",
            ],
            &[
                "",
                "connect_timeout=10",
                "options='-c a=b'",
                "application_name = x",
            ],
        ];
        let bases = [
            pairs[0].len(),
            pairs[1].len(),
            pairs[2].len(),
            pairs[3].len(),
            16,
            3,
        ];
        for n in 0..bases.iter().product() {
            let c = choices(n, &bases);
            let kept: Vec<String> = (0..4)
                .map(|i| pairs[i][c[i]].to_string())
                .filter(|p| !p.is_empty())
                .collect();
            let taken: Vec<_> = (0..tls_params.len())
                .filter(|i| c[4] & (1 << i) != 0)
                .map(|i| format!("{} = {}", tls_params[i].0, tls_params[i].2))
                .collect();
            let mut params = kept.clone();
            let at = [0, params.len(), 1][c[5]];
            params.splice(at..at, taken.iter().cloned());
            let with = params.join("  ");
            let expected = kept.join(" ").parse::<tokio_postgres::Config>().ok();
            let (rest, tls) = take_tls_params(&with).unwrap();
            assert_eq!(
                rest.parse::<tokio_postgres::Config>().ok(),
                expected,
                "{with}"
            );
            assert!(expected.is_some(), "{with}");
            assert_eq!(tls.mode.is_some(), c[4] & 1 != 0, "{with}");
            if c[4] & 2 != 0 {
                assert_eq!(tls.root_cert.as_deref(), Some("/etc/my ca.pem"), "{with}");
            }
            compared += 1;
        }
        assert!(compared > 10_000, "only {compared} compared");
    }

    fn mode(url: &str) -> Result<SslMode, Error> {
        url.parse::<Database>().map(|d| d.config.get_ssl_mode())
    }

    #[test]
    fn reads_sslmode_as_libpq_does() {
        assert_eq!(mode("postgres://db").unwrap(), SslMode::Prefer);
        assert_eq!(mode("host=db sslmode=disable").unwrap(), SslMode::Disable);
        assert_eq!(
            mode("postgres://db?sslmode=require").unwrap(),
            SslMode::Require
        );
        assert_eq!(
            mode("postgres://db?sslmode=verify-full&sslrootcert=system").unwrap(),
            SslMode::Require
        );
        assert_eq!(
            mode("postgres://db?sslrootcert=system").unwrap(),
            SslMode::Require
        );
        assert!(matches!(
            mode("postgres://db?sslmode=require&sslrootcert=system"),
            Err(Error::WeakerThanSystem { mode }) if mode == "require"
        ));
        assert!(matches!(
            mode("postgres://db?sslmode=verify-ca"),
            Err(Error::VerifyCaWithoutRoot)
        ));
        assert!(matches!(
            mode("postgres://db?sslmode=allow"),
            Err(Error::Value { .. })
        ));
        assert!(matches!(
            mode("postgres://db?sslcert=/c.pem"),
            Err(Error::Unpaired("sslcert", "sslkey"))
        ));
        assert!(matches!(
            mode("postgres://db?sslmode=require&sslrootcert=/nonexistent.pem"),
            Err(Error::File { .. })
        ));
    }

    /// Runs against the database `DATABASE_URL` names, alone and through a pool; without one it
    /// checks nothing.
    #[tokio::test]
    async fn connects() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return;
        };
        use diesel_async::SimpleAsyncConnection;
        let mut conn = connect(&url).await.unwrap();
        conn.batch_execute("SELECT 1").await.unwrap();
        let database: Database = url.parse().unwrap();
        let pool = diesel_async::pooled_connection::deadpool::Pool::builder(database.manager())
            .build()
            .unwrap();
        pool.get()
            .await
            .unwrap()
            .batch_execute("SELECT 1")
            .await
            .unwrap();
    }
}
