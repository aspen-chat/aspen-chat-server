//! The patterns of `regex` parameters: the subset of regular expressions that Rust's `regex`
//! crate, which the server matches with, and JavaScript's `u`-mode `RegExp`, which Aspen's
//! clients check a typed value with, can be made to read alike.
//!
//! A pattern is parsed (`regex-syntax`'s AST) and every node is either of a kind both engines
//! give one meaning, or one of `\d`, `\w`, `\s`, `\b` (with their negations) and `.`, which mean
//! on the server what Unicode says and which the client rewrites to classes that say the same
//! (`client/packages/app/src/features/commands/pattern.ts`). Anything else is refused by name.
//! `spec/bot_command_pattern_vectors.json` holds patterns, values, and refusals that both this
//! module's tests and the client's pass.

use crate::t;
use regex_syntax::ast::{self, Ast};
use std::borrow::Cow;

/// The longest a pattern may be, in characters.
pub const MAX_PATTERN_CHARS: usize = 256;
/// What a pattern's compiled program may use, in bytes, which keeps any pattern quick to
/// check. A Unicode class such as `\w` takes about 50 KiB each time it is written or repeated
/// by a count, so this leaves room for some dozens of them.
const PATTERN_SIZE_LIMIT: usize = 2 << 20;

/// The characters a backslash may stand before as themselves in both engines; `-` joins them
/// inside a class.
const ESCAPABLE: &str = "^$\\.*+?()[]{}|/";

/// Why a pattern is not of the subset. `construct` is the pattern's own text for what is
/// refused, and `instead` the spelling to use.
#[derive(Debug, Clone, PartialEq, Eq, strum::IntoStaticStr)]
#[strum(serialize_all = "camelCase")]
pub enum PatternError {
    TooLong,
    /// It does not parse; `at` is the character where reading stopped, counted from one.
    Syntax {
        at: usize,
    },
    LookAround,
    Backreference,
    /// Its compiled program is over `PATTERN_SIZE_LIMIT`, or its groups nest too deep.
    TooLarge,
    Flags {
        construct: String,
    },
    NamedGroup {
        construct: String,
    },
    /// A construct with another spelling that both engines read alike.
    Spelling {
        construct: String,
        instead: String,
    },
    /// `{,n}` and `{}`, which have no least count.
    RepetitionCount,
    PosixClass {
        construct: String,
    },
    ClassSetOperation {
        construct: String,
    },
    NestedClass,
    /// A `\p` whose name or value is not in the lists below, or which is not `name` or
    /// `name=value`.
    Property {
        construct: String,
    },
    /// A word boundary other than `\b` and `\B`.
    WordBoundary {
        construct: String,
    },
    /// An assertion inside a class, where JavaScript reads `\b` as a backspace.
    ClassAssertion {
        construct: String,
    },
    NestedRepetition {
        construct: String,
    },
    RepeatedAssertion {
        construct: String,
    },
}

impl PatternError {
    /// What is wrong and what to write instead, for the bot's developer.
    pub fn detail(&self) -> Cow<'static, str> {
        match self {
            Self::TooLong => t!("botPatternTooLong", max = MAX_PATTERN_CHARS),
            Self::Syntax { at } => t!("botPatternSyntax", at = at),
            Self::LookAround => t!("botPatternLookAround"),
            Self::Backreference => t!("botPatternBackreference"),
            Self::TooLarge => t!("botPatternTooLarge"),
            Self::Flags { construct } => t!("botPatternFlags", construct = construct),
            Self::NamedGroup { construct } => t!("botPatternNamedGroup", construct = construct),
            Self::Spelling { construct, instead } => {
                t!(
                    "botPatternSpelling",
                    construct = construct,
                    instead = instead
                )
            }
            Self::RepetitionCount => t!("botPatternRepetitionCount"),
            Self::PosixClass { construct } => t!("botPatternPosixClass", construct = construct),
            Self::ClassSetOperation { construct } => {
                t!("botPatternClassSetOperation", construct = construct)
            }
            Self::NestedClass => t!("botPatternNestedClass"),
            Self::Property { construct } => t!("botPatternProperty", construct = construct),
            Self::WordBoundary { construct } => t!("botPatternWordBoundary", construct = construct),
            Self::ClassAssertion { construct } => {
                t!("botPatternClassAssertion", construct = construct)
            }
            Self::NestedRepetition { construct } => {
                t!("botPatternNestedRepetition", construct = construct)
            }
            Self::RepeatedAssertion { construct } => {
                t!("botPatternRepeatedAssertion", construct = construct)
            }
        }
    }
}

/// A pattern as a bot publishes it: of the subset, and compiled to match a value whole.
pub fn check(pattern: &str) -> Result<regex::Regex, PatternError> {
    if pattern.chars().count() > MAX_PATTERN_CHARS {
        return Err(PatternError::TooLong);
    }
    // Parsed on its own, so that a pattern closing a group it never opened is refused rather
    // than read against the group `compile` wraps it in.
    let parsed = ast::parse::Parser::new()
        .parse(pattern)
        .map_err(|error| parse_error(pattern, &error))?;
    ast::visit(&parsed, Subset { pattern })?;
    compile(pattern).ok_or(PatternError::TooLarge)
}

/// A pattern of a list already kept, compiled without the subset's check. That check is made
/// when a list is published, so a command kept under a wider subset takes the values it took
/// until its bot publishes again.
pub fn published(pattern: &str) -> Option<regex::Regex> {
    compile(pattern)
}

fn compile(pattern: &str) -> Option<regex::Regex> {
    regex::RegexBuilder::new(&format!("^(?:{pattern})$"))
        .size_limit(PATTERN_SIZE_LIMIT)
        .dfa_size_limit(PATTERN_SIZE_LIMIT)
        .build()
        .ok()
}

fn parse_error(pattern: &str, error: &ast::Error) -> PatternError {
    let span = error.span();
    let construct = || text(pattern, span).to_string();
    match error.kind() {
        ast::ErrorKind::UnsupportedLookAround => PatternError::LookAround,
        ast::ErrorKind::UnsupportedBackreference => PatternError::Backreference,
        ast::ErrorKind::RepetitionCountDecimalEmpty => PatternError::RepetitionCount,
        ast::ErrorKind::ClassEscapeInvalid => PatternError::ClassAssertion {
            construct: construct(),
        },
        _ => PatternError::Syntax {
            at: pattern[..span.start.offset].chars().count() + 1,
        },
    }
}

fn text<'p>(pattern: &'p str, span: &ast::Span) -> &'p str {
    &pattern[span.start.offset..span.end.offset]
}

/// Refuses the first node that is not of the subset.
struct Subset<'p> {
    pattern: &'p str,
}

impl Subset<'_> {
    fn text(&self, span: &ast::Span) -> String {
        text(self.pattern, span).to_string()
    }

    fn spelling(&self, span: &ast::Span, instead: impl Into<String>) -> PatternError {
        PatternError::Spelling {
            construct: self.text(span),
            instead: instead.into(),
        }
    }

    /// A literal, outside a class or as a member or the end of a range inside one. An unescaped
    /// `-` inside a class is `class`'s to judge.
    fn literal(&self, literal: &ast::Literal, in_class: bool) -> Result<(), PatternError> {
        use ast::{HexLiteralKind, LiteralKind, SpecialLiteralKind};
        let c = literal.c;
        let by_code = || self.spelling(&literal.span, format!("\\u{{{:X}}}", u32::from(c)));
        match &literal.kind {
            // JavaScript refuses a `]` or a `}` that closes nothing; Rust reads the character.
            LiteralKind::Verbatim if c == ']' || (c == '}' && !in_class) => {
                Err(self.spelling(&literal.span, format!("\\{c}")))
            }
            LiteralKind::Verbatim => Ok(()),
            LiteralKind::Meta | LiteralKind::Superfluous
                if ESCAPABLE.contains(c) || (in_class && c == '-') =>
            {
                Ok(())
            }
            LiteralKind::Meta | LiteralKind::Superfluous => Err(self.spelling(&literal.span, c)),
            LiteralKind::HexFixed(HexLiteralKind::X | HexLiteralKind::UnicodeShort)
            | LiteralKind::HexBrace(HexLiteralKind::UnicodeShort) => Ok(()),
            LiteralKind::HexFixed(HexLiteralKind::UnicodeLong)
            | LiteralKind::HexBrace(HexLiteralKind::X | HexLiteralKind::UnicodeLong)
            | LiteralKind::Octal => Err(by_code()),
            LiteralKind::Special(
                SpecialLiteralKind::FormFeed
                | SpecialLiteralKind::Tab
                | SpecialLiteralKind::LineFeed
                | SpecialLiteralKind::CarriageReturn
                | SpecialLiteralKind::VerticalTab,
            ) => Ok(()),
            LiteralKind::Special(SpecialLiteralKind::Bell) => {
                Err(self.spelling(&literal.span, "\\x07"))
            }
            LiteralKind::Special(SpecialLiteralKind::Space) => Err(self.spelling(&literal.span, c)),
        }
    }

    fn property(&self, class: &ast::ClassUnicode) -> Result<(), PatternError> {
        use ast::{ClassUnicodeKind, ClassUnicodeOpKind};
        let listed = |list: &[&str], name: &str| list.binary_search(&name).is_ok();
        let known = match &class.kind {
            ClassUnicodeKind::OneLetter(letter) => {
                let p = if class.negated { 'P' } else { 'p' };
                return Err(self.spelling(&class.span, format!("\\{p}{{{letter}}}")));
            }
            ClassUnicodeKind::Named(name) => {
                listed(GENERAL_CATEGORIES, name) || listed(BINARY_PROPERTIES, name)
            }
            ClassUnicodeKind::NamedValue {
                op: ClassUnicodeOpKind::Equal,
                name,
                value,
            } => match name.as_str() {
                "General_Category" | "gc" => listed(GENERAL_CATEGORIES, value),
                "Script" | "sc" | "Script_Extensions" | "scx" => listed(SCRIPTS, value),
                _ => false,
            },
            ClassUnicodeKind::NamedValue { .. } => false,
        };
        if known {
            Ok(())
        } else {
            Err(PatternError::Property {
                construct: self.text(&class.span),
            })
        }
    }

    /// The unescaped `-`s among a class's members: JavaScript reads one between two members as
    /// a range, so one stands only first or last.
    fn class(&self, class: &ast::ClassBracketed) -> Result<(), PatternError> {
        let members = match &class.kind {
            ast::ClassSet::Item(ast::ClassSetItem::Union(union)) => union.items.as_slice(),
            ast::ClassSet::Item(item) => std::slice::from_ref(item),
            // `visit_class_set_binary_op_pre` refuses it.
            ast::ClassSet::BinaryOp(_) => &[],
        };
        let inner = members.get(1..members.len().saturating_sub(1));
        for member in inner.unwrap_or_default() {
            if let ast::ClassSetItem::Literal(literal) = member
                && literal.kind == ast::LiteralKind::Verbatim
                && literal.c == '-'
            {
                return Err(self.spelling(&literal.span, "\\-"));
            }
        }
        Ok(())
    }
}

impl ast::Visitor for Subset<'_> {
    type Output = ();
    type Err = PatternError;

    fn finish(self) -> Result<(), PatternError> {
        Ok(())
    }

    fn visit_pre(&mut self, node: &Ast) -> Result<(), PatternError> {
        use ast::{AssertionKind, GroupKind, RepetitionKind};
        match node {
            Ast::Empty(_)
            | Ast::Dot(_)
            | Ast::ClassPerl(_)
            | Ast::Alternation(_)
            | Ast::Concat(_) => Ok(()),
            Ast::Flags(flags) => Err(PatternError::Flags {
                construct: self.text(&flags.span),
            }),
            Ast::Literal(literal) => self.literal(literal, false),
            Ast::Assertion(assertion) => match assertion.kind {
                AssertionKind::StartLine
                | AssertionKind::EndLine
                | AssertionKind::WordBoundary
                | AssertionKind::NotWordBoundary => Ok(()),
                AssertionKind::StartText => Err(self.spelling(&assertion.span, "^")),
                AssertionKind::EndText => Err(self.spelling(&assertion.span, "$")),
                AssertionKind::WordBoundaryStart
                | AssertionKind::WordBoundaryEnd
                | AssertionKind::WordBoundaryStartAngle
                | AssertionKind::WordBoundaryEndAngle
                | AssertionKind::WordBoundaryStartHalf
                | AssertionKind::WordBoundaryEndHalf => Err(PatternError::WordBoundary {
                    construct: self.text(&assertion.span),
                }),
            },
            Ast::ClassUnicode(class) => self.property(class),
            Ast::ClassBracketed(class) => self.class(class),
            Ast::Repetition(repetition) => {
                let construct = || self.text(&repetition.span);
                match &*repetition.ast {
                    // JavaScript repeats only a character, a class, or a group.
                    Ast::Repetition(_) => {
                        return Err(PatternError::NestedRepetition {
                            construct: construct(),
                        });
                    }
                    Ast::Assertion(_) => {
                        return Err(PatternError::RepeatedAssertion {
                            construct: construct(),
                        });
                    }
                    _ => {}
                }
                let op = &repetition.op;
                let written = text(self.pattern, &op.span);
                // Rust reads spaces around a count's numbers; JavaScript does not.
                if matches!(op.kind, RepetitionKind::Range(_))
                    && written.contains(char::is_whitespace)
                {
                    let instead: String = written.split_whitespace().collect();
                    return Err(self.spelling(&op.span, instead));
                }
                Ok(())
            }
            Ast::Group(group) => match &group.kind {
                GroupKind::CaptureIndex(_) => Ok(()),
                GroupKind::NonCapturing(flags) if flags.items.is_empty() => Ok(()),
                GroupKind::NonCapturing(flags) => Err(PatternError::Flags {
                    construct: format!("(?{}:", text(self.pattern, &flags.span)),
                }),
                GroupKind::CaptureName {
                    starts_with_p,
                    name,
                } => Err(PatternError::NamedGroup {
                    construct: format!(
                        "(?{}<{}>",
                        if *starts_with_p { "P" } else { "" },
                        name.name
                    ),
                }),
            },
        }
    }

    fn visit_class_set_item_pre(&mut self, item: &ast::ClassSetItem) -> Result<(), PatternError> {
        use ast::ClassSetItem;
        match item {
            ClassSetItem::Empty(_) | ClassSetItem::Perl(_) | ClassSetItem::Union(_) => Ok(()),
            ClassSetItem::Literal(literal) => self.literal(literal, true),
            ClassSetItem::Range(range) => {
                for end in [&range.start, &range.end] {
                    // Rust reads `[--a]` as three members, never as a range from `-`.
                    if end.kind == ast::LiteralKind::Verbatim && end.c == '-' {
                        return Err(self.spelling(&end.span, "\\-"));
                    }
                    self.literal(end, true)?;
                }
                Ok(())
            }
            ClassSetItem::Ascii(class) => Err(PatternError::PosixClass {
                construct: self.text(&class.span),
            }),
            ClassSetItem::Unicode(class) => self.property(class),
            ClassSetItem::Bracketed(_) => Err(PatternError::NestedClass),
        }
    }

    fn visit_class_set_binary_op_pre(
        &mut self,
        op: &ast::ClassSetBinaryOp,
    ) -> Result<(), PatternError> {
        let operator = match op.kind {
            ast::ClassSetBinaryOpKind::Intersection => "&&",
            ast::ClassSetBinaryOpKind::Difference => "--",
            ast::ClassSetBinaryOpKind::SymmetricDifference => "~~",
        };
        Err(PatternError::ClassSetOperation {
            construct: operator.to_string(),
        })
    }
}

// The `\p{…}` names both engines know, spelled as JavaScript requires them (Rust also reads them
// loosely). Each is sorted by code point for `binary_search`; the vector file lists the same
// names, which the client's tests compile.

/// General categories, by their short and long names.
pub const GENERAL_CATEGORIES: &[&str] = &[
    "C",
    "Cased_Letter",
    "Cc",
    "Cf",
    "Close_Punctuation",
    "Cn",
    "Co",
    "Connector_Punctuation",
    "Control",
    "Currency_Symbol",
    "Dash_Punctuation",
    "Decimal_Number",
    "Enclosing_Mark",
    "Final_Punctuation",
    "Format",
    "Initial_Punctuation",
    "L",
    "LC",
    "Letter",
    "Letter_Number",
    "Line_Separator",
    "Ll",
    "Lm",
    "Lo",
    "Lowercase_Letter",
    "Lt",
    "Lu",
    "M",
    "Mark",
    "Math_Symbol",
    "Mc",
    "Me",
    "Mn",
    "Modifier_Letter",
    "Modifier_Symbol",
    "N",
    "Nd",
    "Nl",
    "No",
    "Nonspacing_Mark",
    "Number",
    "Open_Punctuation",
    "Other",
    "Other_Letter",
    "Other_Number",
    "Other_Punctuation",
    "Other_Symbol",
    "P",
    "Paragraph_Separator",
    "Pc",
    "Pd",
    "Pe",
    "Pf",
    "Pi",
    "Po",
    "Private_Use",
    "Ps",
    "Punctuation",
    "S",
    "Sc",
    "Separator",
    "Sk",
    "Sm",
    "So",
    "Space_Separator",
    "Spacing_Mark",
    "Symbol",
    "Titlecase_Letter",
    "Unassigned",
    "Uppercase_Letter",
    "Z",
    "Zl",
    "Zp",
    "Zs",
];

/// Binary properties, by their long names and short aliases.
pub const BINARY_PROPERTIES: &[&str] = &[
    "AHex",
    "ASCII",
    "ASCII_Hex_Digit",
    "Alpha",
    "Alphabetic",
    "Any",
    "Assigned",
    "Bidi_C",
    "Bidi_Control",
    "Bidi_M",
    "Bidi_Mirrored",
    "CI",
    "CWCF",
    "CWCM",
    "CWL",
    "CWT",
    "CWU",
    "Case_Ignorable",
    "Cased",
    "Changes_When_Casefolded",
    "Changes_When_Casemapped",
    "Changes_When_Lowercased",
    "Changes_When_Titlecased",
    "Changes_When_Uppercased",
    "DI",
    "Dash",
    "Default_Ignorable_Code_Point",
    "Dep",
    "Deprecated",
    "Dia",
    "Diacritic",
    "EBase",
    "EComp",
    "EMod",
    "EPres",
    "Emoji",
    "Emoji_Component",
    "Emoji_Modifier",
    "Emoji_Modifier_Base",
    "Emoji_Presentation",
    "Ext",
    "ExtPict",
    "Extended_Pictographic",
    "Extender",
    "Gr_Base",
    "Gr_Ext",
    "Grapheme_Base",
    "Grapheme_Extend",
    "Hex",
    "Hex_Digit",
    "IDC",
    "IDS",
    "IDSB",
    "IDST",
    "IDS_Binary_Operator",
    "IDS_Trinary_Operator",
    "ID_Continue",
    "ID_Start",
    "Ideo",
    "Ideographic",
    "Join_C",
    "Join_Control",
    "LOE",
    "Logical_Order_Exception",
    "Lower",
    "Lowercase",
    "Math",
    "NChar",
    "Noncharacter_Code_Point",
    "Pat_Syn",
    "Pat_WS",
    "Pattern_Syntax",
    "Pattern_White_Space",
    "QMark",
    "Quotation_Mark",
    "RI",
    "Radical",
    "Regional_Indicator",
    "SD",
    "STerm",
    "Sentence_Terminal",
    "Soft_Dotted",
    "Term",
    "Terminal_Punctuation",
    "UIdeo",
    "Unified_Ideograph",
    "Upper",
    "Uppercase",
    "VS",
    "Variation_Selector",
    "White_Space",
    "XIDC",
    "XIDS",
    "XID_Continue",
    "XID_Start",
    "space",
];

/// Scripts, by their names and four-letter codes.
pub const SCRIPTS: &[&str] = &[
    "Adlam",
    "Adlm",
    "Aghb",
    "Ahom",
    "Anatolian_Hieroglyphs",
    "Arab",
    "Arabic",
    "Armenian",
    "Armi",
    "Armn",
    "Avestan",
    "Avst",
    "Bali",
    "Balinese",
    "Bamu",
    "Bamum",
    "Bass",
    "Bassa_Vah",
    "Batak",
    "Batk",
    "Beng",
    "Bengali",
    "Bhaiksuki",
    "Bhks",
    "Bopo",
    "Bopomofo",
    "Brah",
    "Brahmi",
    "Brai",
    "Braille",
    "Bugi",
    "Buginese",
    "Buhd",
    "Buhid",
    "Cakm",
    "Canadian_Aboriginal",
    "Cans",
    "Cari",
    "Carian",
    "Caucasian_Albanian",
    "Chakma",
    "Cham",
    "Cher",
    "Cherokee",
    "Chorasmian",
    "Chrs",
    "Common",
    "Copt",
    "Coptic",
    "Cpmn",
    "Cprt",
    "Cuneiform",
    "Cypriot",
    "Cypro_Minoan",
    "Cyrillic",
    "Cyrl",
    "Deseret",
    "Deva",
    "Devanagari",
    "Diak",
    "Dives_Akuru",
    "Dogr",
    "Dogra",
    "Dsrt",
    "Dupl",
    "Duployan",
    "Egyp",
    "Egyptian_Hieroglyphs",
    "Elba",
    "Elbasan",
    "Elym",
    "Elymaic",
    "Ethi",
    "Ethiopic",
    "Gara",
    "Garay",
    "Geor",
    "Georgian",
    "Glag",
    "Glagolitic",
    "Gong",
    "Gonm",
    "Goth",
    "Gothic",
    "Gran",
    "Grantha",
    "Greek",
    "Grek",
    "Gujarati",
    "Gujr",
    "Gukh",
    "Gunjala_Gondi",
    "Gurmukhi",
    "Guru",
    "Gurung_Khema",
    "Han",
    "Hang",
    "Hangul",
    "Hani",
    "Hanifi_Rohingya",
    "Hano",
    "Hanunoo",
    "Hatr",
    "Hatran",
    "Hebr",
    "Hebrew",
    "Hira",
    "Hiragana",
    "Hluw",
    "Hmng",
    "Hmnp",
    "Hung",
    "Imperial_Aramaic",
    "Inherited",
    "Inscriptional_Pahlavi",
    "Inscriptional_Parthian",
    "Ital",
    "Java",
    "Javanese",
    "Kaithi",
    "Kali",
    "Kana",
    "Kannada",
    "Katakana",
    "Kawi",
    "Kayah_Li",
    "Khar",
    "Kharoshthi",
    "Khitan_Small_Script",
    "Khmer",
    "Khmr",
    "Khoj",
    "Khojki",
    "Khudawadi",
    "Kirat_Rai",
    "Kits",
    "Knda",
    "Krai",
    "Kthi",
    "Lana",
    "Lao",
    "Laoo",
    "Latin",
    "Latn",
    "Lepc",
    "Lepcha",
    "Limb",
    "Limbu",
    "Lina",
    "Linb",
    "Linear_A",
    "Linear_B",
    "Lisu",
    "Lyci",
    "Lycian",
    "Lydi",
    "Lydian",
    "Mahajani",
    "Mahj",
    "Maka",
    "Makasar",
    "Malayalam",
    "Mand",
    "Mandaic",
    "Mani",
    "Manichaean",
    "Marc",
    "Marchen",
    "Masaram_Gondi",
    "Medefaidrin",
    "Medf",
    "Meetei_Mayek",
    "Mend",
    "Mende_Kikakui",
    "Merc",
    "Mero",
    "Meroitic_Cursive",
    "Meroitic_Hieroglyphs",
    "Miao",
    "Mlym",
    "Modi",
    "Mong",
    "Mongolian",
    "Mro",
    "Mroo",
    "Mtei",
    "Mult",
    "Multani",
    "Myanmar",
    "Mymr",
    "Nabataean",
    "Nag_Mundari",
    "Nagm",
    "Nand",
    "Nandinagari",
    "Narb",
    "Nbat",
    "New_Tai_Lue",
    "Newa",
    "Nko",
    "Nkoo",
    "Nshu",
    "Nushu",
    "Nyiakeng_Puachue_Hmong",
    "Ogam",
    "Ogham",
    "Ol_Chiki",
    "Ol_Onal",
    "Olck",
    "Old_Hungarian",
    "Old_Italic",
    "Old_North_Arabian",
    "Old_Permic",
    "Old_Persian",
    "Old_Sogdian",
    "Old_South_Arabian",
    "Old_Turkic",
    "Old_Uyghur",
    "Onao",
    "Oriya",
    "Orkh",
    "Orya",
    "Osage",
    "Osge",
    "Osma",
    "Osmanya",
    "Ougr",
    "Pahawh_Hmong",
    "Palm",
    "Palmyrene",
    "Pau_Cin_Hau",
    "Pauc",
    "Perm",
    "Phag",
    "Phags_Pa",
    "Phli",
    "Phlp",
    "Phnx",
    "Phoenician",
    "Plrd",
    "Prti",
    "Psalter_Pahlavi",
    "Qaac",
    "Qaai",
    "Rejang",
    "Rjng",
    "Rohg",
    "Runic",
    "Runr",
    "Samaritan",
    "Samr",
    "Sarb",
    "Saur",
    "Saurashtra",
    "Sgnw",
    "Sharada",
    "Shavian",
    "Shaw",
    "Shrd",
    "Sidd",
    "Siddham",
    "SignWriting",
    "Sind",
    "Sinh",
    "Sinhala",
    "Sogd",
    "Sogdian",
    "Sogo",
    "Sora",
    "Sora_Sompeng",
    "Soyo",
    "Soyombo",
    "Sund",
    "Sundanese",
    "Sunu",
    "Sunuwar",
    "Sylo",
    "Syloti_Nagri",
    "Syrc",
    "Syriac",
    "Tagalog",
    "Tagb",
    "Tagbanwa",
    "Tai_Le",
    "Tai_Tham",
    "Tai_Viet",
    "Takr",
    "Takri",
    "Tale",
    "Talu",
    "Tamil",
    "Taml",
    "Tang",
    "Tangsa",
    "Tangut",
    "Tavt",
    "Telu",
    "Telugu",
    "Tfng",
    "Tglg",
    "Thaa",
    "Thaana",
    "Thai",
    "Tibetan",
    "Tibt",
    "Tifinagh",
    "Tirh",
    "Tirhuta",
    "Tnsa",
    "Todhri",
    "Todr",
    "Toto",
    "Tulu_Tigalari",
    "Tutg",
    "Ugar",
    "Ugaritic",
    "Vai",
    "Vaii",
    "Vith",
    "Vithkuqi",
    "Wancho",
    "Wara",
    "Warang_Citi",
    "Wcho",
    "Xpeo",
    "Xsux",
    "Yezi",
    "Yezidi",
    "Yi",
    "Yiii",
    "Zanabazar_Square",
    "Zanb",
    "Zinh",
    "Zyyy",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Vectors {
        accepted: Vec<Accepted>,
        refused: Vec<Refused>,
        properties: Properties,
    }

    #[derive(Deserialize)]
    struct Accepted {
        pattern: String,
        matches: Vec<String>,
        rejects: Vec<String>,
    }

    #[derive(Deserialize)]
    struct Refused {
        pattern: String,
        reason: String,
        construct: Option<String>,
        instead: Option<String>,
        at: Option<usize>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Properties {
        general_categories: Vec<String>,
        binary_properties: Vec<String>,
        scripts: Vec<String>,
    }

    fn vectors() -> Vectors {
        serde_json::from_str(include_str!(
            "../../../../spec/bot_command_pattern_vectors.json"
        ))
        .unwrap()
    }

    /// What a refusal names, as the vectors write it.
    fn named(error: &PatternError) -> (Option<&str>, Option<&str>, Option<usize>) {
        match error {
            PatternError::TooLong
            | PatternError::LookAround
            | PatternError::Backreference
            | PatternError::TooLarge
            | PatternError::RepetitionCount
            | PatternError::NestedClass => (None, None, None),
            PatternError::Syntax { at } => (None, None, Some(*at)),
            PatternError::Spelling { construct, instead } => (Some(construct), Some(instead), None),
            PatternError::Flags { construct }
            | PatternError::NamedGroup { construct }
            | PatternError::PosixClass { construct }
            | PatternError::ClassSetOperation { construct }
            | PatternError::Property { construct }
            | PatternError::WordBoundary { construct }
            | PatternError::ClassAssertion { construct }
            | PatternError::NestedRepetition { construct }
            | PatternError::RepeatedAssertion { construct } => (Some(construct), None, None),
        }
    }

    #[test]
    fn the_shared_vectors_match_and_are_refused_as_they_say() {
        let vectors = vectors();
        let mut wrong = Vec::new();
        for Accepted {
            pattern,
            matches,
            rejects,
        } in &vectors.accepted
        {
            let compiled = match check(pattern) {
                Ok(compiled) => compiled,
                Err(error) => {
                    wrong.push(format!("{pattern:?} is refused: {error:?}"));
                    continue;
                }
            };
            for (values, expected) in [(matches, true), (rejects, false)] {
                for value in values {
                    if compiled.is_match(value) != expected {
                        wrong.push(format!("{pattern:?} on {value:?} is not {expected}"));
                    }
                }
            }
        }
        for refused in &vectors.refused {
            let pattern = &refused.pattern;
            let expected = (
                refused.construct.as_deref(),
                refused.instead.as_deref(),
                refused.at,
            );
            match check(pattern) {
                Ok(_) => wrong.push(format!("{pattern:?} is accepted")),
                Err(error) => {
                    if <&str>::from(&error) != refused.reason || named(&error) != expected {
                        wrong.push(format!("{pattern:?} is refused as {error:?}"));
                    }
                    assert!(!error.detail().is_empty());
                }
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// The lists here are the vector file's, which the client's tests compile, and every name
    /// compiles on the server in every form the subset gives it.
    #[test]
    fn the_property_names_are_the_shared_ones_and_all_compile() {
        let Properties {
            general_categories,
            binary_properties,
            scripts,
        } = vectors().properties;
        assert_eq!(GENERAL_CATEGORIES, general_categories);
        assert_eq!(BINARY_PROPERTIES, binary_properties);
        assert_eq!(SCRIPTS, scripts);
        for list in [GENERAL_CATEGORIES, BINARY_PROPERTIES, SCRIPTS] {
            assert!(list.is_sorted(), "binary_search needs them sorted");
        }
        let compiles = |pattern: String| assert!(check(&pattern).is_ok(), "{pattern}");
        for name in GENERAL_CATEGORIES {
            compiles(format!("\\p{{{name}}}"));
            compiles(format!("\\P{{gc={name}}}"));
            compiles(format!("[\\p{{General_Category={name}}}]"));
        }
        for name in BINARY_PROPERTIES {
            compiles(format!("\\p{{{name}}}"));
        }
        for name in SCRIPTS {
            for property in ["Script", "sc", "Script_Extensions", "scx"] {
                compiles(format!("\\p{{{property}={name}}}"));
            }
        }
    }

    #[test]
    fn a_pattern_is_bounded_in_length_and_in_compiled_size() {
        assert!(check(&"a".repeat(MAX_PATTERN_CHARS)).is_ok());
        assert!(check(&"é".repeat(MAX_PATTERN_CHARS)).is_ok());
        assert_eq!(
            check(&"a".repeat(MAX_PATTERN_CHARS + 1)).unwrap_err(),
            PatternError::TooLong
        );
        assert_eq!(
            check("(?:\\w{100}){100}").unwrap_err(),
            PatternError::TooLarge
        );
        let nested = format!("{}a{}", "(".repeat(120), ")".repeat(120));
        assert!(check(&nested).is_ok());
    }

    /// Names JavaScript knows too, but which the server's engine does not, or which hold
    /// surrogates, which no value the server receives can.
    #[test]
    fn a_property_only_javascript_knows_is_refused() {
        for pattern in ["\\p{Cs}", "\\p{Script=Zzzz}", "\\p{CWKCF}", "\\p{punct}"] {
            assert!(
                matches!(check(pattern), Err(PatternError::Property { .. })),
                "{pattern}"
            );
        }
    }

    /// A kept list's pattern is compiled whether or not the subset has it.
    #[test]
    fn a_published_pattern_outside_the_subset_still_matches() {
        for (pattern, value) in [
            ("\\pL+", "été"),
            ("\\p{Greek}+", "αβγ"),
            ("[a-z&&[^x]]", "b"),
            ("\\x{41}\\#", "A#"),
            ("(?<n>a)\\b{end}", "a"),
            ("a**", "aa"),
        ] {
            assert!(check(pattern).is_err(), "{pattern}");
            let compiled = published(pattern).expect(pattern);
            assert!(compiled.is_match(value), "{pattern} on {value}");
            assert!(!compiled.is_match("!"), "{pattern}");
        }
        assert!(published("(").is_none());
    }
}
