from __future__ import annotations

from collections.abc import Callable, Iterable
from typing import TYPE_CHECKING

from aspen_client.types import UserProfile

if TYPE_CHECKING:
    from aspen_client.api_client import AspenApiClient, TaskSpawner


class UserDirectory:
    """Caches per-user profile + presence status; dedupes background fetches.

    Mirrors ``IconCache`` for the user-profile / presence side of the
    state ``ChatWindow`` would otherwise carry directly. The render
    hot-path (per-row author lookup, presence-dot lookup) calls
    ``get_profile`` / ``get_status`` and must not block; missing
    profiles trigger a background fetch via ``TaskSpawner.run`` and the
    registered ``on_profile_loaded`` callback fires on the GUI thread
    once the bytes land so the caller (the chat window) can patch live
    widgets.

    Owning this state outside the ``ChatWindow`` keeps the
    profile/presence cache off the layout object and lets
    ``_reset_client_state`` discard the entire cache via ``clear()``
    rather than touching three private maps.
    """

    def __init__(
        self,
        api: "AspenApiClient",
        tasks: "TaskSpawner",
        on_profile_loaded: Callable[[str], None],
    ) -> None:
        self._api = api
        self._tasks = tasks
        self._on_profile_loaded = on_profile_loaded
        self._profiles: dict[str, UserProfile] = {}
        self._statuses: dict[str, str] = {}
        self._pending_profile_fetches: set[str] = set()

    def clear(self) -> None:
        """Drop every cached profile / presence entry and forget pending fetches.

        Used during a state resync: the three maps must be wiped together
        so a stale profile or presence dot never shadows a fresh one
        across a reconnect.
        """
        self._profiles.clear()
        self._statuses.clear()
        self._pending_profile_fetches.clear()

    def get_profile(self, user_id: str) -> UserProfile | None:
        return self._profiles.get(user_id)

    def upsert_profile(self, profile: UserProfile) -> None:
        """Insert a profile we already loaded synchronously.

        Used when a community-users fetch returns a list of profiles --
        we don't want to re-request them one-by-one when the user opens
        a channel.
        """
        self._profiles[profile.id] = profile

    def get_status(self, user_id: str) -> str:
        """Return cached presence status, defaulting to ``"offline"``."""
        return self._statuses.get(user_id, "offline")

    def set_status(self, user_id: str, status: str) -> None:
        """Update presence from a ``userStatus`` WebSocket event."""
        self._statuses[user_id] = status

    def request_profiles(self, user_ids: Iterable[str]) -> None:
        """Kick off async profile loads for any ids we don't know yet.

        Returns immediately. The dedupe set guarantees one in-flight
        fetch per user no matter how many call sites trigger a render
        for the same author. The ``on_profile_loaded`` callback fires
        on the GUI thread when each fetch completes so the caller can
        re-render any rows showing that user.
        """
        for user_id in user_ids:
            if user_id in self._profiles:
                continue
            if user_id in self._pending_profile_fetches:
                continue
            self._pending_profile_fetches.add(user_id)
            self._tasks.run(
                self._api.read_user_profile(user_id),
                on_success=lambda profile, uid=user_id: self._on_profile_resolved(
                    uid, profile
                ),
                on_failure=lambda _exc, uid=user_id: self._pending_profile_fetches.discard(
                    uid
                ),
            )

    def _on_profile_resolved(self, user_id: str, profile: UserProfile) -> None:
        self._pending_profile_fetches.discard(user_id)
        self._profiles[user_id] = profile
        self._on_profile_loaded(user_id)
