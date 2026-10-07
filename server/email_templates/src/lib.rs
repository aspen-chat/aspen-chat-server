//! Mail as `app::email` writes it: every kind is one [`Letter`], set in
//! `server/templates/email/letter.html` and `letter.txt` ([`Html`] and [`Text`]).

use askama::Template;

/// Everything a piece of mail may say, top to bottom.
#[derive(Debug, Default)]
pub struct Letter {
    pub subject: String,
    /// The language it is written in, and its direction.
    pub lang: String,
    pub dir: &'static str,
    /// The deployment's name, above it.
    pub deployment: String,
    pub title: String,
    pub intro: Vec<String>,
    /// A code to type, shown large.
    pub code: Option<String>,
    /// A newsletter's body, as HTML made from its Markdown and as the Markdown itself.
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub sections: Vec<Section>,
    pub outro: Vec<String>,
    pub footer: Vec<String>,
    pub unsubscribe: Option<Unsubscribe>,
}

/// A digest's conversation: where it is, what arrived there, and how much more did.
#[derive(Debug)]
pub struct Section {
    pub heading: String,
    pub link: String,
    pub items: Vec<Item>,
    pub more: Option<String>,
}

#[derive(Debug)]
pub struct Item {
    pub author: String,
    pub time: String,
    pub text: String,
}

#[derive(Debug)]
pub struct Unsubscribe {
    pub label: String,
    pub url: String,
}

#[derive(Template)]
#[template(path = "email/letter.html")]
pub struct Html<'a> {
    pub letter: &'a Letter,
}

#[derive(Template)]
#[template(path = "email/letter.txt")]
pub struct Text<'a> {
    pub letter: &'a Letter,
}
