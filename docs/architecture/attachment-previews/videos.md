# Videos

A video's preview is its poster (`video::make`). Apps show it with a play control and play the original only when asked, so nothing of a video is fetched until the reader wants it. Video is not transcoded.

## The poster

1. Copy the original into a scratch directory (under `TMPDIR`).
2. Run the operator's `ffprobe` and `ffmpeg` (`[media.previews] ffmpeg`, `ffprobe`) as processes of their own, under the sandbox below.
3. Take the frame a second in, or the middle of a shorter video, turned upright as it is decoded.
4. Make it into a picture's preview (see [What a preview is](preview-format.md)).

## Videos with no poster

- HDR video. Its frame flattened to SDR without tone mapping would look washed out.
- A video larger than `max_video_bytes`.

## The sandbox

| Restriction | How |
| --- | --- |
| Read only the copied file | `-protocol_whitelist file` |
| Only the containers videos are sent in | `-format_whitelist`, `video::CONTAINERS` |
| Only the codecs such files hold, sound and subtitles included | `-codec_whitelist`, `video::DECODERS` |
| Frames of at most `max_picture_pixels` | `-max_pixels` |
| One thread | |
| No environment but `PATH` | |
| Address space | `ffmpeg_memory_mib` |
| CPU time | A minute |
| Files written | None |
| Core dumps | None |
| Open files | 64 |
| Output read for the frame | Up to `max_picture_bytes`, the process killed past it |
| Output read from `ffprobe` | A megabyte, the process killed past it |
| Wall time | Each run stopped after a minute |

The resource limits are set as they start (`video::limit`, `setrlimit` between fork and exec).

- The decoder list includes sound and subtitles because reading a file's streams opens their decoders too.
- The protocol and format lists mean a file made to look like a playlist cannot have them fetch addresses or read other files.

## Servers without `ffmpeg`

A server on which `ffmpeg` or `ffprobe` will not run makes previews of pictures only. It says so in its log when it starts, and leaves videos to the servers that can.

[Design notes](design-notes.md#videos)
