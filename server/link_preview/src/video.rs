//! Video embeds: the `VIDEO_PROVIDERS` allowlist, the players built from page URLs, and the
//! providers' oEmbed answers.

use crate::fetch::{MAX_METADATA_BYTES, http_client, read_capped};
use aspen_wire::link_preview::VideoEmbed;
use tracing::debug;
use url::Url;

/// How a provider's player URL is obtained.
enum Player {
    /// The `src` of the iframe in the provider's oEmbed `html`, kept only when its host is one
    /// of `hosts`; `rewrite` substitutes a reduced-tracking player domain where one exists.
    Oembed {
        hosts: &'static [&'static str],
        rewrite: Option<(&'static str, &'static str)>,
    },
    /// Built from the page URL by `build`, for providers without oEmbed or whose oEmbed
    /// answer carries no iframe. `width` and `height` give the player's aspect ratio.
    FromUrl {
        build: fn(&Url) -> Option<String>,
        width: u32,
        height: u32,
    },
}

/// A video site whose players may be framed by clients. The allowlist is applied twice: the
/// server only calls the oEmbed endpoints named here, never one a page advertises, and only
/// produces player URLs on hosts named here or built by the functions here. Clients keep the
/// matching list of player hosts and frame nothing else.
pub struct VideoProvider {
    /// Hosts of pages the provider serves videos on, matched exactly.
    page_hosts: &'static [&'static str],
    /// Host suffixes matched with a leading dot, for providers that give each account a
    /// subdomain.
    page_host_suffixes: &'static [&'static str],
    /// The provider's oEmbed endpoint, which takes `url` and `format=json`, used for the title,
    /// thumbnail, and provider name, and for the player when `player` is `Player::Oembed`.
    oembed_endpoint: Option<&'static str>,
    player: Player,
}

const VIDEO_PROVIDERS: &[VideoProvider] = &[
    VideoProvider {
        page_hosts: &[
            "www.youtube.com",
            "youtube.com",
            "m.youtube.com",
            "youtu.be",
        ],
        page_host_suffixes: &[],
        oembed_endpoint: Some("https://www.youtube.com/oembed"),
        player: Player::Oembed {
            hosts: &["www.youtube.com", "www.youtube-nocookie.com"],
            rewrite: Some(("www.youtube.com", "www.youtube-nocookie.com")),
        },
    },
    VideoProvider {
        page_hosts: &["vimeo.com", "www.vimeo.com"],
        page_host_suffixes: &[],
        oembed_endpoint: Some("https://vimeo.com/api/oembed.json"),
        player: Player::Oembed {
            hosts: &["player.vimeo.com"],
            rewrite: None,
        },
    },
    VideoProvider {
        page_hosts: &["www.dailymotion.com", "dailymotion.com", "dai.ly"],
        page_host_suffixes: &[],
        oembed_endpoint: Some("https://www.dailymotion.com/services/oembed"),
        player: Player::Oembed {
            hosts: &["geo.dailymotion.com", "www.dailymotion.com"],
            rewrite: None,
        },
    },
    VideoProvider {
        page_hosts: &["streamable.com", "www.streamable.com"],
        page_host_suffixes: &[],
        oembed_endpoint: Some("https://api.streamable.com/oembed.json"),
        player: Player::Oembed {
            hosts: &["streamable.com"],
            rewrite: None,
        },
    },
    VideoProvider {
        page_hosts: &["wistia.com", "www.wistia.com"],
        page_host_suffixes: &[".wistia.com"],
        oembed_endpoint: Some("https://fast.wistia.com/oembed"),
        player: Player::Oembed {
            hosts: &["fast.wistia.net"],
            rewrite: None,
        },
    },
    VideoProvider {
        page_hosts: &["www.ted.com", "ted.com"],
        page_host_suffixes: &[],
        oembed_endpoint: Some("https://www.ted.com/services/v1/oembed.json"),
        player: Player::Oembed {
            hosts: &["embed.ted.com"],
            rewrite: None,
        },
    },
    // TikTok's oEmbed answer is a script-driven blockquote, not a frame; the player is built
    // from the video id and the oEmbed answer supplies the title and thumbnail.
    VideoProvider {
        page_hosts: &["www.tiktok.com", "tiktok.com"],
        page_host_suffixes: &[],
        oembed_endpoint: Some("https://www.tiktok.com/oembed"),
        player: Player::FromUrl {
            build: tiktok_player,
            width: 9,
            height: 16,
        },
    },
    VideoProvider {
        page_hosts: &[
            "www.twitch.tv",
            "twitch.tv",
            "m.twitch.tv",
            "clips.twitch.tv",
        ],
        page_host_suffixes: &[],
        oembed_endpoint: None,
        player: Player::FromUrl {
            build: twitch_player,
            width: 16,
            height: 9,
        },
    },
    VideoProvider {
        page_hosts: &["www.bilibili.com", "bilibili.com", "m.bilibili.com"],
        page_host_suffixes: &[],
        oembed_endpoint: None,
        player: Player::FromUrl {
            build: bilibili_player,
            width: 16,
            height: 9,
        },
    },
    VideoProvider {
        page_hosts: &["v.youku.com", "www.youku.com", "youku.com"],
        page_host_suffixes: &[],
        oembed_endpoint: None,
        player: Player::FromUrl {
            build: youku_player,
            width: 16,
            height: 9,
        },
    },
    VideoProvider {
        page_hosts: &[
            "www.nicovideo.jp",
            "nicovideo.jp",
            "sp.nicovideo.jp",
            "nico.ms",
        ],
        page_host_suffixes: &[],
        oembed_endpoint: None,
        player: Player::FromUrl {
            build: niconico_player,
            width: 16,
            height: 9,
        },
    },
    VideoProvider {
        page_hosts: &[
            "vk.com",
            "www.vk.com",
            "m.vk.com",
            "vkvideo.ru",
            "www.vkvideo.ru",
        ],
        page_host_suffixes: &[],
        oembed_endpoint: None,
        player: Player::FromUrl {
            build: vk_player,
            width: 16,
            height: 9,
        },
    },
    VideoProvider {
        page_hosts: &["rutube.ru", "www.rutube.ru"],
        page_host_suffixes: &[],
        oembed_endpoint: None,
        player: Player::FromUrl {
            build: rutube_player,
            width: 16,
            height: 9,
        },
    },
    VideoProvider {
        page_hosts: &["ok.ru", "www.ok.ru", "m.ok.ru"],
        page_host_suffixes: &[],
        oembed_endpoint: None,
        player: Player::FromUrl {
            build: okru_player,
            width: 16,
            height: 9,
        },
    },
    VideoProvider {
        page_hosts: &["tv.naver.com"],
        page_host_suffixes: &[],
        oembed_endpoint: None,
        player: Player::FromUrl {
            build: naver_player,
            width: 16,
            height: 9,
        },
    },
    VideoProvider {
        page_hosts: &["www.aparat.com", "aparat.com"],
        page_host_suffixes: &[],
        oembed_endpoint: None,
        player: Player::FromUrl {
            build: aparat_player,
            width: 16,
            height: 9,
        },
    },
];

pub fn video_provider_for(url: &Url) -> Option<&'static VideoProvider> {
    let host = url.host_str()?.to_ascii_lowercase();
    VIDEO_PROVIDERS.iter().find(|p| {
        p.page_hosts.contains(&host.as_str())
            || p.page_host_suffixes
                .iter()
                .any(|suffix| host.ends_with(suffix))
    })
}

/// Path segments of a URL, without empty ones.
fn segments(url: &Url) -> Vec<&str> {
    url.path_segments()
        .map(|s| s.filter(|seg| !seg.is_empty()).collect())
        .unwrap_or_default()
}

fn is_id(value: &str, extra: &[char]) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
}

/// `www.tiktok.com/@user/video/{id}` to the official player.
fn tiktok_player(url: &Url) -> Option<String> {
    let segs = segments(url);
    let at = segs.iter().position(|s| *s == "video")?;
    let id = segs.get(at + 1)?;
    is_id(id, &[]).then(|| format!("https://www.tiktok.com/player/v1/{id}"))
}

/// Twitch videos, live channels, and clips. The player requires a `parent` query parameter
/// naming the embedding site, which only the client knows; it appends one before framing.
fn twitch_player(url: &Url) -> Option<String> {
    let host = url.host_str()?.to_ascii_lowercase();
    let segs = segments(url);
    if host == "clips.twitch.tv" {
        let [slug, ..] = segs.as_slice() else {
            return None;
        };
        return is_id(slug, &['-', '_'])
            .then(|| format!("https://clips.twitch.tv/embed?clip={slug}"));
    }
    match segs.as_slice() {
        ["videos", id] if is_id(id, &[]) => Some(format!("https://player.twitch.tv/?video={id}")),
        [channel, "clip", slug] if is_id(channel, &['_']) && is_id(slug, &['-', '_']) => {
            Some(format!("https://clips.twitch.tv/embed?clip={slug}"))
        }
        [channel] if is_id(channel, &['_']) => {
            Some(format!("https://player.twitch.tv/?channel={channel}"))
        }
        _ => None,
    }
}

/// `www.bilibili.com/video/{BV… or av…}` to the official player.
fn bilibili_player(url: &Url) -> Option<String> {
    let segs = segments(url);
    let at = segs.iter().position(|s| *s == "video")?;
    let id = segs.get(at + 1)?;
    if let Some(aid) = id.strip_prefix("av")
        && aid.bytes().all(|b| b.is_ascii_digit())
        && !aid.is_empty()
    {
        return Some(format!("https://player.bilibili.com/player.html?aid={aid}"));
    }
    (id.starts_with("BV") && is_id(id, &[]))
        .then(|| format!("https://player.bilibili.com/player.html?bvid={id}"))
}

/// `v.youku.com/v_show/id_{id}.html` to the official player.
fn youku_player(url: &Url) -> Option<String> {
    let last = *segments(url).last()?;
    let id = last.strip_prefix("id_")?.strip_suffix(".html")?;
    is_id(id, &['=']).then(|| format!("https://player.youku.com/embed/{id}"))
}

/// `www.nicovideo.jp/watch/{sm…}` or `nico.ms/{sm…}` to the official embed.
fn niconico_player(url: &Url) -> Option<String> {
    let segs = segments(url);
    let id = match segs.as_slice() {
        ["watch", id] => id,
        [id] if url.host_str() == Some("nico.ms") => id,
        _ => return None,
    };
    is_id(id, &[]).then(|| format!("https://embed.nicovideo.jp/watch/{id}"))
}

/// `vk.com/video{owner}_{id}` or `?z=video{owner}_{id}…` to the official external player.
fn vk_player(url: &Url) -> Option<String> {
    let mut candidates: Vec<String> = segments(url).iter().map(|s| (*s).to_owned()).collect();
    if let Some(z) = url.query_pairs().find(|(k, _)| k == "z") {
        candidates.push(z.1.split('/').next().unwrap_or_default().to_owned());
    }
    for candidate in candidates {
        if let Some(rest) = candidate.strip_prefix("video")
            && let Some((owner, id)) = rest.split_once('_')
            && owner
                .trim_start_matches('-')
                .bytes()
                .all(|b| b.is_ascii_digit())
            && !owner.trim_start_matches('-').is_empty()
            && !id.is_empty()
            && id.bytes().all(|b| b.is_ascii_digit())
        {
            return Some(format!("https://vk.com/video_ext.php?oid={owner}&id={id}"));
        }
    }
    None
}

/// `rutube.ru/video/{hash}/` to the official embed.
fn rutube_player(url: &Url) -> Option<String> {
    let segs = segments(url);
    match segs.as_slice() {
        ["video", id] | ["shorts", id] if is_id(id, &[]) => {
            Some(format!("https://rutube.ru/play/embed/{id}"))
        }
        _ => None,
    }
}

/// `ok.ru/video/{id}` to the official embed.
fn okru_player(url: &Url) -> Option<String> {
    match segments(url).as_slice() {
        ["video", id] if id.bytes().all(|b| b.is_ascii_digit()) && !id.is_empty() => {
            Some(format!("https://ok.ru/videoembed/{id}"))
        }
        _ => None,
    }
}

/// `tv.naver.com/v/{id}` to the official embed.
fn naver_player(url: &Url) -> Option<String> {
    match segments(url).as_slice() {
        ["v", id] if id.bytes().all(|b| b.is_ascii_digit()) && !id.is_empty() => {
            Some(format!("https://tv.naver.com/embed/{id}"))
        }
        _ => None,
    }
}

/// `www.aparat.com/v/{hash}` to the official frame.
fn aparat_player(url: &Url) -> Option<String> {
    match segments(url).as_slice() {
        ["v", hash] if is_id(hash, &[]) => Some(format!(
            "https://www.aparat.com/video/video/embed/videohash/{hash}/vt/frame"
        )),
        _ => None,
    }
}

/// The parts of an oEmbed answer a preview uses.
#[derive(Debug, Default, serde::Deserialize)]
struct OembedResponse {
    #[serde(rename = "type")]
    kind: String,
    title: Option<String>,
    provider_name: Option<String>,
    thumbnail_url: Option<String>,
    html: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

pub struct VideoEmbedMetadata {
    pub embed: VideoEmbed,
    pub title: Option<String>,
    pub provider_name: Option<String>,
    pub thumbnail_url: Option<String>,
}

async fn fetch_oembed(endpoint: &str, url: &Url) -> Option<OembedResponse> {
    let endpoint =
        Url::parse_with_params(endpoint, [("url", url.as_str()), ("format", "json")]).ok()?;
    let response = match http_client().get(endpoint.as_str()).send().await {
        Ok(r) => r,
        Err(e) => {
            debug!(
                url = url.as_str(),
                error = e.to_string(),
                "oEmbed fetch failed"
            );
            return None;
        }
    };
    if !response.status().is_success() {
        return None;
    }
    let bytes = read_capped(response, MAX_METADATA_BYTES).await?;
    serde_json::from_slice(&bytes).ok()
}

pub async fn fetch_video_embed(provider: &VideoProvider, url: &Url) -> Option<VideoEmbedMetadata> {
    let oembed = match provider.oembed_endpoint {
        Some(endpoint) => fetch_oembed(endpoint, url).await,
        None => None,
    };
    video_embed_from(provider, url, oembed)
}

/// Combines the page URL and the provider's oEmbed answer, if any, into a player plus the
/// metadata the answer carried. Only `video` answers count; a `rich` or `link` answer yields
/// no player even if it contains a frame.
fn video_embed_from(
    provider: &VideoProvider,
    url: &Url,
    oembed: Option<OembedResponse>,
) -> Option<VideoEmbedMetadata> {
    let oembed = oembed.filter(|o| o.kind == "video");
    let embed = match provider.player {
        Player::Oembed { hosts, rewrite } => {
            let oembed = oembed.as_ref()?;
            let src = player_src_from_html(hosts, rewrite, oembed.html.as_deref()?)?;
            let (width, height) = (oembed.width?, oembed.height?);
            if width == 0 || height == 0 {
                return None;
            }
            VideoEmbed { src, width, height }
        }
        Player::FromUrl {
            build,
            width,
            height,
        } => VideoEmbed {
            src: build(url)?,
            width,
            height,
        },
    };
    let oembed = oembed.unwrap_or_default();
    Some(VideoEmbedMetadata {
        embed,
        title: oembed.title,
        provider_name: oembed.provider_name,
        thumbnail_url: oembed.thumbnail_url,
    })
}

/// The `src` of the first `<iframe>` in an oEmbed `html` snippet, kept only when it is an
/// `https` URL on one of `hosts`. Nothing else from the snippet is used.
fn player_src_from_html(
    hosts: &[&str],
    rewrite: Option<(&str, &str)>,
    html: &str,
) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let tag_start = lower.find("<iframe")?;
    let tag_end = lower[tag_start..].find('>')? + tag_start;
    let tag = &lower[tag_start..tag_end];
    let attr = tag.find("src=")? + 4;
    let quote = *tag.as_bytes().get(attr)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let value_start = tag_start + attr + 1;
    let value_len = html[value_start..].find(quote as char)?;
    let raw = &html[value_start..value_start + value_len];
    let mut src = Url::parse(raw).ok()?;
    if src.scheme() != "https" {
        return None;
    }
    let host = src.host_str()?.to_ascii_lowercase();
    if !hosts.contains(&host.as_str()) {
        return None;
    }
    if let Some((from, to)) = rewrite
        && host == from
    {
        src.set_host(Some(to)).ok()?;
    }
    Some(src.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn youtube() -> &'static VideoProvider {
        video_provider_for(&Url::parse("https://youtu.be/dQw4w9WgXcQ").unwrap()).unwrap()
    }

    fn youtube_hosts() -> (
        &'static [&'static str],
        Option<(&'static str, &'static str)>,
    ) {
        match youtube().player {
            Player::Oembed { hosts, rewrite } => (hosts, rewrite),
            Player::FromUrl { .. } => unreachable!(),
        }
    }

    #[test]
    fn video_provider_matches_only_allowlisted_page_hosts() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert!(video_provider_for(&u("https://www.youtube.com/watch?v=x")).is_some());
        assert!(video_provider_for(&u("https://vimeo.com/1")).is_some());
        assert!(video_provider_for(&u("https://acme.wistia.com/medias/abc")).is_some());
        assert!(video_provider_for(&u("https://youtube.com.evil.test/watch")).is_none());
        assert!(video_provider_for(&u("https://notwistia.com/medias/abc")).is_none());
        assert!(video_provider_for(&u("https://example.org/video")).is_none());
    }

    #[test]
    fn player_src_is_taken_from_the_iframe_only_when_it_is_the_provider_player() {
        let (hosts, rewrite) = youtube_hosts();
        let html = r#"<iframe width="200" height="113" src="https://www.youtube.com/embed/dQw4w9WgXcQ?feature=oembed" frameborder="0" allow="autoplay" allowfullscreen></iframe>"#;
        assert_eq!(
            player_src_from_html(hosts, rewrite, html).as_deref(),
            Some("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?feature=oembed"),
        );
        for bad in [
            r#"<iframe src='http://www.youtube.com/embed/x'></iframe>"#,
            r#"<iframe src="https://evil.test/embed/x"></iframe>"#,
            r#"<blockquote class="tiktok-embed"></blockquote><script src="https://www.tiktok.com/embed.js"></script>"#,
            r#"<iframe src="javascript:alert(1)"></iframe>"#,
        ] {
            assert!(player_src_from_html(hosts, rewrite, bad).is_none(), "{bad}");
        }
    }

    /// A YouTube oEmbed answer as served on 2026-09-25, trimmed to the fields used.
    #[test]
    fn video_embed_from_a_youtube_oembed_answer() {
        let url = Url::parse("https://youtu.be/dQw4w9WgXcQ").unwrap();
        let json = r#"{"title":"Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)","author_name":"Rick Astley","type":"video","height":113,"width":200,"version":"1.0","provider_name":"YouTube","provider_url":"https://www.youtube.com/","thumbnail_url":"https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg","html":"\u003ciframe width=\u0022200\u0022 height=\u0022113\u0022 src=\u0022https://www.youtube.com/embed/dQw4w9WgXcQ?feature=oembed\u0022 frameborder=\u00220\u0022 allowfullscreen\u003e\u003c/iframe\u003e"}"#;
        let oembed: OembedResponse = serde_json::from_str(json).unwrap();
        let video = video_embed_from(youtube(), &url, Some(oembed)).unwrap();
        assert_eq!(
            video.embed,
            VideoEmbed {
                src: "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?feature=oembed"
                    .to_string(),
                width: 200,
                height: 113,
            }
        );
        assert_eq!(video.provider_name.as_deref(), Some("YouTube"));
        assert!(video.title.as_deref().unwrap().starts_with("Rick Astley"));
        let not_video: OembedResponse = serde_json::from_str(
            r#"{"type":"rich","html":"<iframe src=\"https://www.youtube.com/embed/x\"></iframe>","width":1,"height":1}"#,
        )
        .unwrap();
        assert!(video_embed_from(youtube(), &url, Some(not_video)).is_none());
        // Without an oEmbed answer an oEmbed-player provider has no player.
        assert!(video_embed_from(youtube(), &url, None).is_none());
    }

    #[test]
    fn players_built_from_page_urls() {
        let cases: &[(&str, Option<&str>)] = &[
            (
                "https://www.tiktok.com/@scout2015/video/6718335390845095173",
                Some("https://www.tiktok.com/player/v1/6718335390845095173"),
            ),
            ("https://www.tiktok.com/@scout2015", None),
            (
                "https://www.twitch.tv/videos/1234567890",
                Some("https://player.twitch.tv/?video=1234567890"),
            ),
            (
                "https://www.twitch.tv/some_channel",
                Some("https://player.twitch.tv/?channel=some_channel"),
            ),
            (
                "https://clips.twitch.tv/FunnyClip-abc_123",
                Some("https://clips.twitch.tv/embed?clip=FunnyClip-abc_123"),
            ),
            (
                "https://www.twitch.tv/some_channel/clip/FunnyClip-abc",
                Some("https://clips.twitch.tv/embed?clip=FunnyClip-abc"),
            ),
            ("https://www.twitch.tv/directory/game/x", None),
            (
                "https://www.bilibili.com/video/BV1GJ411x7h7/?p=2",
                Some("https://player.bilibili.com/player.html?bvid=BV1GJ411x7h7"),
            ),
            (
                "https://www.bilibili.com/video/av170001",
                Some("https://player.bilibili.com/player.html?aid=170001"),
            ),
            ("https://www.bilibili.com/bangumi/play/ss1", None),
            (
                "https://v.youku.com/v_show/id_XMzk4NzY3NDE2OA==.html",
                Some("https://player.youku.com/embed/XMzk4NzY3NDE2OA=="),
            ),
            (
                "https://www.nicovideo.jp/watch/sm9",
                Some("https://embed.nicovideo.jp/watch/sm9"),
            ),
            (
                "https://nico.ms/sm9",
                Some("https://embed.nicovideo.jp/watch/sm9"),
            ),
            (
                "https://vk.com/video-22822305_456241864",
                Some("https://vk.com/video_ext.php?oid=-22822305&id=456241864"),
            ),
            (
                "https://vk.com/feed?z=video-22822305_456241864%2Fpl_wall",
                Some("https://vk.com/video_ext.php?oid=-22822305&id=456241864"),
            ),
            (
                "https://vkvideo.ru/video12345_67890",
                Some("https://vk.com/video_ext.php?oid=12345&id=67890"),
            ),
            ("https://vk.com/durov", None),
            (
                "https://rutube.ru/video/c7a3dd4b2d4a1a7a8a5e4c5d6f7e8a9b/",
                Some("https://rutube.ru/play/embed/c7a3dd4b2d4a1a7a8a5e4c5d6f7e8a9b"),
            ),
            (
                "https://ok.ru/video/1234567890",
                Some("https://ok.ru/videoembed/1234567890"),
            ),
            (
                "https://tv.naver.com/v/12345678",
                Some("https://tv.naver.com/embed/12345678"),
            ),
            (
                "https://www.aparat.com/v/abc12",
                Some("https://www.aparat.com/video/video/embed/videohash/abc12/vt/frame"),
            ),
        ];
        for (page, expected) in cases {
            let url = Url::parse(page).unwrap();
            let provider = video_provider_for(&url).expect(page);
            let got = video_embed_from(provider, &url, None).map(|v| v.embed.src);
            assert_eq!(got.as_deref(), *expected, "{page}");
        }
    }

    #[test]
    fn every_built_player_is_https_on_a_fixed_host() {
        for page in [
            "https://www.tiktok.com/@a/video/1",
            "https://www.twitch.tv/videos/1",
            "https://www.bilibili.com/video/BV1",
            "https://v.youku.com/v_show/id_X.html",
            "https://www.nicovideo.jp/watch/sm1",
            "https://vk.com/video1_2",
            "https://rutube.ru/video/a/",
            "https://ok.ru/video/1",
            "https://tv.naver.com/v/1",
            "https://www.aparat.com/v/a",
        ] {
            let url = Url::parse(page).unwrap();
            let provider = video_provider_for(&url).expect(page);
            let src = video_embed_from(provider, &url, None)
                .expect(page)
                .embed
                .src;
            let parsed = Url::parse(&src).unwrap();
            assert_eq!(parsed.scheme(), "https", "{page}");
            assert!(parsed.host_str().is_some(), "{page}");
        }
    }
}
