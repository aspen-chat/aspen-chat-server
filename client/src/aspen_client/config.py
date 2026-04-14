from __future__ import annotations

from dataclasses import dataclass
import os


def _to_bool(value: str | None, default: bool) -> bool:
    if value is None:
        return default
    return value.strip().lower() in {"1", "true", "yes", "on"}


@dataclass(slots=True)
class ClientConfig:
    api_base_url: str
    ws_url: str
    verify_tls: bool

    @classmethod
    def from_env(cls) -> "ClientConfig":
        api_base_url = os.getenv("ASPEN_API_BASE_URL", "https://127.0.0.1:443").rstrip("/")
        ws_url = os.getenv("ASPEN_WS_URL", "")
        if not ws_url:
            if api_base_url.startswith("https://"):
                ws_url = f"wss://{api_base_url[len('https://'):]}/event_stream"
            elif api_base_url.startswith("http://"):
                ws_url = f"ws://{api_base_url[len('http://'):]}/event_stream"
            else:
                ws_url = f"{api_base_url}/event_stream"
        verify_tls = _to_bool(os.getenv("ASPEN_VERIFY_TLS"), default=False)
        return cls(api_base_url=api_base_url, ws_url=ws_url, verify_tls=verify_tls)
