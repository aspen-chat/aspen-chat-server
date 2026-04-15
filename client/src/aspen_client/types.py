from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime


@dataclass(slots=True)
class LoginSession:
    user_id: str
    refresh_token: str
    session_token: str
    session_token_expires: datetime


@dataclass(slots=True)
class Community:
    id: str
    name: str
    icon: str | None


@dataclass(slots=True)
class Channel:
    id: str
    name: str
    ty: str
    community: str | None
    parent_category: str | None
    sort_index: int


@dataclass(slots=True)
class Message:
    id: str
    author: str
    channel_id: str
    timestamp: datetime
    content: str
    attachments: list[str]


@dataclass(slots=True)
class UserProfile:
    id: str
    name: str
    icon: str | None
