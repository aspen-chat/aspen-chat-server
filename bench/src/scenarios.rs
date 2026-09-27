//! The built-in scenarios (`bench/profiles/`): profiles for common traffic shapes, named on the
//! command line instead of a file. Each targets a deployment on this machine; override with
//! `--api` and `--metrics`, or copy one and edit it.

pub const ALL: [(&str, &str); 10] = [
    ("smoke", include_str!("../profiles/smoke.toml")),
    ("small-group", include_str!("../profiles/small-group.toml")),
    (
        "gaming-community",
        include_str!("../profiles/gaming-community.toml"),
    ),
    (
        "public-community",
        include_str!("../profiles/public-community.toml"),
    ),
    (
        "announcement-fanout",
        include_str!("../profiles/announcement-fanout.toml"),
    ),
    (
        "reconnect-storm",
        include_str!("../profiles/reconnect-storm.toml"),
    ),
    (
        "bootstrap-storm",
        include_str!("../profiles/bootstrap-storm.toml"),
    ),
    (
        "voice-evening",
        include_str!("../profiles/voice-evening.toml"),
    ),
    ("soak", include_str!("../profiles/soak.toml")),
    ("chaos", include_str!("../profiles/chaos.toml")),
];

pub fn find(name: &str) -> Option<&'static str> {
    ALL.iter().find(|(n, _)| *n == name).map(|(_, text)| *text)
}

#[cfg(test)]
mod tests {
    use crate::profile::Profile;

    #[test]
    fn every_scenario_is_a_valid_profile_with_a_seedable_population() {
        for (name, text) in super::ALL {
            let profile = Profile::from_toml(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(profile.name, name);
            profile
                .seed_plan("t")
                .validate(500)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }
}
