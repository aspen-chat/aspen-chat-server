//! Writing out a piece of mail in its reader's language: every kind is one [`Letter`], set in
//! `templates/email/letter.html` and `letter.txt` and sent as both, so a reader whose mail
//! program shows no HTML reads the same thing. Mail that belongs to a list carries the RFC 8058
//! one-click unsubscribe headers and a link to leave it at its foot (`api::email`).

use super::outbox::{Factor, List, Mail, Recipient};
use super::{Mailer, newsletter};
use crate::context::GlobalServerContext;
use crate::t;
use askama::Template;
use aspen_email_templates::{Html, Text, Unsubscribe};
pub(super) use aspen_email_templates::{Item, Letter, Section};
use aspen_schema::user;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use lettre::Message;
use lettre::message::header::{HeaderName, HeaderValue};
use lettre::message::{Mailbox, MultiPart};

/// The message `mail` makes for `recipient`. `None` when there is nothing to send: a newsletter
/// post that no longer exists.
pub(super) async fn message(
    state: &GlobalServerContext,
    mailer: &Mailer,
    recipient: &Recipient,
    mail: &Mail,
) -> crate::Result<Option<Message>> {
    let post = match mail {
        Mail::Newsletter { post, .. } => match newsletter::read_post(state, *post).await? {
            Some(post) => Some(post),
            None => return Ok(None),
        },
        _ => None,
    };
    let name: String = {
        let mut conn = state.connection_pool.get().await?;
        user::table
            .select(user::name)
            .filter(user::id.eq(recipient.user))
            .first(conn.as_mut())
            .await?
    };
    let deployment = state.settings().name().to_string();
    let locale = crate::locale::negotiate(&recipient.locale);
    let unsubscribe = mail.list().map(|list| {
        let list_name: &'static str = match list {
            List::Newsletter => "newsletter",
            List::Digest => "digest",
        };
        format!(
            "{}/email/unsubscribe?list={list_name}&token={}",
            mailer.public_url, recipient.unsubscribe_token
        )
    });
    let letter = crate::locale::scope(locale, async {
        let mut letter = write(mail, &name, &deployment, post.as_ref());
        letter.lang = locale.to_string();
        letter.dir = if locale == crate::locale::MIRRORED {
            "rtl"
        } else {
            "ltr"
        };
        letter.deployment = deployment.clone();
        letter
            .footer
            .push(t!("emailSentBy", deployment = deployment.as_str()).into());
        if let Some(url) = &unsubscribe {
            letter.unsubscribe = Some(Unsubscribe {
                label: t!("emailUnsubscribe").into(),
                url: url.clone(),
            });
        }
        letter
    })
    .await;
    let html = Html { letter: &letter }
        .render()
        .map_err(|e| crate::Error::Email(e.to_string()))?;
    let text = Text { letter: &letter }
        .render()
        .map_err(|e| crate::Error::Email(e.to_string()))?;
    let to = recipient
        .address
        .parse::<Mailbox>()
        .map_err(|e| crate::Error::Email(e.to_string()))?;
    let mut builder = Message::builder()
        .from(mailer.from.clone())
        .to(to)
        .subject(letter.subject.clone());
    if let Some(url) = unsubscribe {
        builder = builder
            .raw_header(HeaderValue::new(
                HeaderName::new_from_ascii_str("List-Unsubscribe"),
                format!("<{url}>"),
            ))
            .raw_header(HeaderValue::new(
                HeaderName::new_from_ascii_str("List-Unsubscribe-Post"),
                "List-Unsubscribe=One-Click".to_string(),
            ));
    }
    let message = builder
        .multipart(MultiPart::alternative_plain_html(text, html))
        .map_err(|e| crate::Error::Email(e.to_string()))?;
    Ok(Some(message))
}

/// What `mail` says to the account named `name`, in the locale in scope.
fn write(
    mail: &Mail,
    name: &str,
    deployment: &str,
    post: Option<&newsletter::NewsletterPost>,
) -> Letter {
    match mail {
        Mail::Verification { code } => Letter {
            subject: t!("emailVerificationSubject", deployment = deployment).into(),
            title: t!("emailVerificationTitle").into(),
            intro: vec![t!("emailVerificationIntro", deployment = deployment).into()],
            code: Some(code.clone()),
            outro: vec![t!("emailIgnoreIfNotYou").into()],
            ..Letter::default()
        },
        Mail::PasswordReset { code } => Letter {
            subject: t!("emailResetSubject", deployment = deployment).into(),
            title: t!("emailResetTitle").into(),
            intro: vec![t!("emailResetIntro", name = name, deployment = deployment).into()],
            code: Some(code.clone()),
            outro: vec![t!("emailResetIgnore").into()],
            ..Letter::default()
        },
        Mail::PasswordWasReset { removed_factors } => {
            let mut intro = vec![
                t!(
                    "emailPasswordWasResetIntro",
                    name = name,
                    deployment = deployment
                )
                .into(),
            ];
            if *removed_factors > 0 {
                intro.push(
                    t!(
                        "emailPasswordWasResetFactorsRemoved",
                        count = removed_factors
                    )
                    .into(),
                );
            }
            Letter {
                subject: t!("emailPasswordWasResetSubject", deployment = deployment).into(),
                title: t!("emailPasswordWasResetTitle").into(),
                intro,
                outro: vec![t!("emailPasswordWasResetNotYou").into()],
                ..Letter::default()
            }
        }
        Mail::PasswordChanged => Letter {
            subject: t!("emailPasswordChangedSubject", deployment = deployment).into(),
            title: t!("emailPasswordChangedTitle").into(),
            intro: vec![
                t!(
                    "emailPasswordChangedIntro",
                    name = name,
                    deployment = deployment
                )
                .into(),
            ],
            outro: vec![t!("emailSecurityNotYou").into()],
            ..Letter::default()
        },
        Mail::SecondFactorAdded { factor } => Letter {
            subject: t!("emailFactorAddedSubject", deployment = deployment).into(),
            title: t!("emailFactorAddedTitle").into(),
            intro: vec![match factor {
                Factor::AuthenticatorApp => {
                    t!("emailFactorAddedApp", name = name, deployment = deployment).into()
                }
                Factor::Passkey { name: passkey } => t!(
                    "emailFactorAddedPasskey",
                    name = name,
                    deployment = deployment,
                    passkey = passkey.as_str()
                )
                .into(),
            }],
            outro: vec![t!("emailSecurityNotYou").into()],
            ..Letter::default()
        },
        Mail::SecondFactorRemoved { factor } => Letter {
            subject: t!("emailFactorRemovedSubject", deployment = deployment).into(),
            title: t!("emailFactorRemovedTitle").into(),
            intro: vec![match factor {
                Factor::AuthenticatorApp => t!(
                    "emailFactorRemovedApp",
                    name = name,
                    deployment = deployment
                )
                .into(),
                Factor::Passkey { name: passkey } => t!(
                    "emailFactorRemovedPasskey",
                    name = name,
                    deployment = deployment,
                    passkey = passkey.as_str()
                )
                .into(),
            }],
            outro: vec![t!("emailSecurityNotYou").into()],
            ..Letter::default()
        },
        Mail::SignInLocked => Letter {
            subject: t!("emailSignInLockedSubject", deployment = deployment).into(),
            title: t!("emailSignInLockedTitle").into(),
            intro: vec![
                t!(
                    "emailSignInLockedIntro",
                    name = name,
                    deployment = deployment
                )
                .into(),
                t!("emailSignInLockedStillWorks").into(),
            ],
            outro: vec![t!("emailSecurityNotYou").into()],
            ..Letter::default()
        },
        Mail::AddressChanged { new_address } => Letter {
            subject: t!("emailAddressChangedSubject", deployment = deployment).into(),
            title: t!("emailAddressChangedTitle").into(),
            intro: vec![match new_address {
                Some(address) => t!(
                    "emailAddressChangedIntro",
                    name = name,
                    deployment = deployment,
                    address = address.as_str()
                )
                .into(),
                None => t!(
                    "emailAddressRemovedIntro",
                    name = name,
                    deployment = deployment
                )
                .into(),
            }],
            outro: vec![t!("emailAddressChangedNotYou").into()],
            ..Letter::default()
        },
        Mail::Digest { digest } => digest.letter(deployment),
        Mail::Newsletter { test, .. } => {
            let post = post.expect("a newsletter's post is read before it is written");
            let subject = if *test {
                t!("newsletterTestSubject", subject = post.subject.as_str()).into()
            } else {
                post.subject.clone()
            };
            Letter {
                title: post.subject.clone(),
                subject,
                body_html: Some(newsletter::html(&post.body)),
                body_text: Some(post.body.clone()),
                footer: vec![t!("newsletterFooter", deployment = deployment).into()],
                ..Letter::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_are_escaped_in_html_and_not_in_text() {
        let letter = Letter {
            subject: "<subject>".to_string(),
            title: "Tom & Jerry <script>".to_string(),
            code: Some("123456".to_string()),
            ..Letter::default()
        };
        let html = Html { letter: &letter }.render().unwrap();
        assert!(html.contains("Tom &#38; Jerry &#60;script&#62;"), "{html}");
        assert!(!html.contains("<script>"));
        let text = Text { letter: &letter }.render().unwrap();
        assert!(text.contains("Tom & Jerry <script>"), "{text}");
        assert!(text.contains("123456"));
    }
}
