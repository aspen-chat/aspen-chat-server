from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone

from pydantic import BaseModel, ConfigDict, Field

@dataclass(slots=True)
class LoginSession:
    """Auth credentials for the current login.

    Kept as a dataclass (rather than a ``BaseModel`` like the records
    below) because there is no server-side wire shape to mirror with
    aliases: every field is constructed explicitly inside
    ``AspenApiClient.login`` from the validated ``gen.LoginResponse``,
    and nothing else ever needs to deserialise a session.
    """

    user_id: str
    refresh_token: str
    session_token: str
    session_token_expires: datetime


# Shared pydantic config for every client-side record:
#
# * ``populate_by_name=True`` lets the same model accept either alias
#   keys (camelCase, as the server speaks them and as the WebSocket
#   event payloads carry them) or python attribute names (snake_case,
#   as the generated REST models emit when ``model_dump`` is called
#   without ``by_alias=True``). This is what lets the REST and event
#   pipelines hand the *same* dict shape to ``model_validate``.
# * ``extra="ignore"`` means the server is free to add fields without
#   the client refusing to deserialise.
# * ``arbitrary_types_allowed=False`` (default) is fine; everything is
#   stdlib types or pydantic-known.
_RECORD_CONFIG = ConfigDict(populate_by_name=True, extra="ignore")


class Community(BaseModel):
    model_config = _RECORD_CONFIG

    id: str
    name: str
    icon: str | None = None


class Channel(BaseModel):
    model_config = _RECORD_CONFIG

    id: str
    name: str
    ty: str
    community: str | None = None
    parent_category: str | None = Field(default=None, alias="parentCategory")
    sort_index: int = Field(default=0, alias="sortIndex")


class Message(BaseModel):
    model_config = _RECORD_CONFIG

    id: str
    author: str
    channel_id: str = Field(alias="channelId")
    # Pydantic parses ISO-8601 strings (including the ``Z`` Zulu suffix)
    # directly into a tz-aware ``datetime`` -- the input may be either a
    # raw string from the server or an already-parsed ``datetime`` from
    # a generated event model.
    timestamp: datetime
    content: str
    attachments: list[str] = Field(default_factory=list)


class UserProfile(BaseModel):
    model_config = _RECORD_CONFIG

    id: str
    name: str
    icon: str | None = None
