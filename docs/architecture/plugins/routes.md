# Routes

Code: `api::plugin::route`.

## The endpoint

`/api/v1/plugins/{plugin}/routes/{*path}`:

- is outside the OpenAPI document;
- is rate limited by `api::rate_limit::PLUGIN_ROUTE` (`plugin_routes` in `rate_limits.toml`), per user per plugin and per plugin.

| Limit | Size |
| --- | --- |
| Request body | 64 KiB |
| Answer | 1 MiB |

## Answer headers

An answer is served on the API's origin.

- A content type a browser would render as a document or run as script becomes `application/octet-stream`.
- Every answer carries `Content-Security-Policy: default-src 'none'; sandbox`.
- Every answer carries `X-Content-Type-Options: nosniff`.

## The host's paths

Routes under `aspen/` (`route::HOST_PREFIX`) are the host's alone. The host calls them for:

- card buttons: `aspen/cards/{message}/{button}` (see [Notices and cards](notices-and-cards.md#cards));
- capability URLs: `aspen/capabilities/{name}` (see [Capability URLs](capability-urls.md)).

### What a person may not ask for

`route::person_may_ask` answers not found to a person's request whose path:

- is under `aspen/`;
- has an empty, `.`, or `..` segment;
- begins with `aspen` in any case, as it stands, with backslashes read as slashes, or percent-decoded again.
