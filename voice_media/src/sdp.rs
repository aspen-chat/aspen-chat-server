//! Format parameters set per section in the offers the server writes, which str0m sets only per
//! connection: a share's sound in stereo at 128 kbit/s without DTX beside a microphone with DTX,
//! and the bitrate Chromium starts a video encoder at. A client reads them from the server's
//! offer, its remote description. str0m only parses what clients answer, so editing its offers'
//! text costs it nothing.

/// Parameters to set on every payload type of one codec in one section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatEdit {
    pub mid: String,
    /// The codec's name in `a=rtpmap`, matched without regard to case.
    pub codec: &'static str,
    pub params: Vec<(&'static str, String)>,
}

/// `sdp` with `edits` applied: each parameter replaces one of the same name, or is added, and a
/// payload type with no `a=fmtp` line is given one.
pub fn edit_format_parameters(sdp: &str, edits: &[FormatEdit]) -> String {
    if edits.is_empty() {
        return sdp.to_owned();
    }
    let mut sections = sdp.split("\r\nm=");
    let mut out = String::with_capacity(sdp.len() + 256);
    out.push_str(sections.next().unwrap_or_default());
    for section in sections {
        out.push_str("\r\nm=");
        let lines: Vec<&str> = section.split("\r\n").collect();
        let mid = lines
            .iter()
            .find_map(|line| line.strip_prefix("a=mid:"))
            .unwrap_or_default();
        let wanted: Vec<&FormatEdit> = edits.iter().filter(|edit| edit.mid == mid).collect();
        if wanted.is_empty() {
            out.push_str(section);
            continue;
        }
        out.push_str(&edit_section(&lines, &wanted).join("\r\n"));
    }
    out
}

fn edit_section(lines: &[&str], edits: &[&FormatEdit]) -> Vec<String> {
    // Each payload type the edits name, with the parameters to give it.
    let mut by_payload_type: Vec<(String, Vec<(&'static str, String)>)> = Vec::new();
    for line in lines {
        let Some(rest) = line.strip_prefix("a=rtpmap:") else {
            continue;
        };
        let Some((payload_type, encoding)) = rest.split_once(' ') else {
            continue;
        };
        let name = encoding.split('/').next().unwrap_or_default();
        for edit in edits.iter().filter(|e| e.codec.eq_ignore_ascii_case(name)) {
            match by_payload_type
                .iter_mut()
                .find(|(pt, _)| pt == payload_type)
            {
                Some((_, params)) => params.extend(edit.params.iter().cloned()),
                None => by_payload_type.push((payload_type.to_owned(), edit.params.clone())),
            }
        }
    }
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + by_payload_type.len());
    let mut given: Vec<&str> = Vec::new();
    for line in lines {
        if let Some(rest) = line.strip_prefix("a=fmtp:") {
            let (payload_type, existing) = rest.split_once(' ').unwrap_or((rest, ""));
            if let Some((_, params)) = by_payload_type.iter().find(|(pt, _)| pt == payload_type) {
                out.push(format!(
                    "a=fmtp:{payload_type} {}",
                    merged(existing, params)
                ));
                given.push(payload_type);
                continue;
            }
        }
        out.push((*line).to_owned());
    }
    // Payload types that had no `a=fmtp` line get one after their `a=rtpmap`.
    for (payload_type, params) in &by_payload_type {
        if given.contains(&payload_type.as_str()) {
            continue;
        }
        let rtpmap = format!("a=rtpmap:{payload_type} ");
        if let Some(at) = out.iter().position(|line| line.starts_with(&rtpmap)) {
            out.insert(
                at + 1,
                format!("a=fmtp:{payload_type} {}", merged("", params)),
            );
        }
    }
    out
}

/// `existing` (`a=b;c=d`) with each of `params` replacing one of its name or added at the end.
fn merged(existing: &str, params: &[(&'static str, String)]) -> String {
    let mut pairs: Vec<(String, String)> = existing
        .split(';')
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((name, value)) => (name.to_owned(), value.to_owned()),
            None => (pair.to_owned(), String::new()),
        })
        .collect();
    for (name, value) in params {
        match pairs
            .iter_mut()
            .find(|(known, _)| known.eq_ignore_ascii_case(name))
        {
            Some((_, old)) => *old = value.clone(),
            None => pairs.push(((*name).to_owned(), value.clone())),
        }
    }
    pairs
        .iter()
        .map(|(name, value)| {
            if value.is_empty() {
                name.clone()
            } else {
                format!("{name}={value}")
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SDP: &str = "v=0\r\no=- 1 1 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\n\
        m=audio 9 UDP/TLS/RTP/SAVPF 100\r\na=mid:0\r\na=rtpmap:100 opus/48000/2\r\n\
        a=fmtp:100 minptime=10;useinbandfec=1;usedtx=1\r\n\
        m=video 9 UDP/TLS/RTP/SAVPF 101 102\r\na=mid:1\r\na=rtpmap:101 VP8/90000\r\n\
        a=rtpmap:102 rtx/90000\r\na=fmtp:102 apt=101\r\n\
        m=audio 9 UDP/TLS/RTP/SAVPF 100\r\na=mid:2\r\na=rtpmap:100 opus/48000/2\r\n\
        a=fmtp:100 minptime=10;useinbandfec=1;usedtx=1\r\n";

    #[test]
    fn a_parameter_replaces_one_of_its_name_in_its_section_alone() {
        let edited = edit_format_parameters(
            SDP,
            &[FormatEdit {
                mid: "2".into(),
                codec: "opus",
                params: vec![("usedtx", "0".into()), ("stereo", "1".into())],
            }],
        );
        assert!(edited.contains(
            "a=mid:2\r\na=rtpmap:100 opus/48000/2\r\na=fmtp:100 minptime=10;useinbandfec=1;usedtx=0;stereo=1\r\n"
        ));
        // Section 0's Opus is left alone.
        assert!(edited.contains("a=mid:0\r\na=rtpmap:100 opus/48000/2\r\na=fmtp:100 minptime=10;useinbandfec=1;usedtx=1\r\n"));
    }

    #[test]
    fn a_payload_type_without_format_parameters_is_given_them() {
        let edited = edit_format_parameters(
            SDP,
            &[FormatEdit {
                mid: "1".into(),
                codec: "VP8",
                params: vec![("x-google-start-bitrate", "10000".into())],
            }],
        );
        assert!(edited.contains(
            "a=rtpmap:101 VP8/90000\r\na=fmtp:101 x-google-start-bitrate=10000\r\na=rtpmap:102 rtx/90000"
        ));
        // RTX keeps its own line untouched.
        assert!(edited.contains("a=fmtp:102 apt=101"));
    }

    #[test]
    fn no_edits_leave_the_text_as_it_was() {
        assert_eq!(edit_format_parameters(SDP, &[]), SDP);
    }
}
