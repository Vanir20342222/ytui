"""Abstract VPN interface."""

from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass
from enum import Enum


class VPNState(str, Enum):
    CONNECTED = "connected"
    DISCONNECTED = "disconnected"
    CONNECTING = "connecting"
    DISCONNECTING = "disconnecting"
    ERROR = "error"
    UNAVAILABLE = "unavailable"


@dataclass
class VPNStatus:
    state: VPNState = VPNState.UNAVAILABLE
    server: str = ""
    country: str = ""
    city: str = ""
    ip: str = ""
    protocol: str = ""
    error_message: str = ""

    @property
    def is_connected(self) -> bool:
        return self.state == VPNState.CONNECTED

    @property
    def display_label(self) -> str:
        if self.state == VPNState.CONNECTED:
            parts = []
            if self.country:
                parts.append(self.country)
            if self.city:
                parts.append(self.city)
            return " · ".join(parts) if parts else "Connected"
        return self.state.value.capitalize()


class VPNProvider(ABC):
    """Abstract VPN provider interface."""

    @abstractmethod
    async def connect(self, server: str | None = None, country: str | None = None) -> VPNStatus:
        ...

    @abstractmethod
    async def disconnect(self) -> VPNStatus:
        ...

    @abstractmethod
    async def get_status(self) -> VPNStatus:
        ...

    @abstractmethod
    async def list_servers(self) -> list[dict[str, str]]:
        ...

    @abstractmethod
    def is_available(self) -> bool:
        ...
