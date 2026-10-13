# Configuration templates

`aspen.toml`, `voice_server.toml`, and NATS's configuration, as every way of deploying from
files writes them: the OpenTofu configurations in `../aws` and `../aws-single`
(`templatefile`), and the box's tool in `../box` (`aspen-box`).

Both fill them the same way, so the templates hold nothing but `${name}` placeholders: no
conditions, loops, or functions. Each value arrives already written as it appears in the file,
which for a string means in quotes (JSON's form of a string, an array, or a number is also
TOML's and NATS's). The two exceptions are `${id}` in `nats_voice_user.tftpl`, which is put
inside the subjects' own quotes, and the blocks a caller writes whole: `${voice_users}` (one
`nats_voice_user.tftpl` per voice server), `${s3_more}` (further lines of `[media.s3]`), and
`${more}` (further tables).

A setting a deployment cannot start without is added here, and then to each caller.
