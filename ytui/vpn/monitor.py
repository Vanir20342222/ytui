"""VPN connection monitor for kill-switch behavior."""

from __future__ import annotations

import asyncio
import logging
from typing import Callable

from ytui.vpn.base import VPNProvider, VPNState

logger = logging.getLogger(__name__)


class VPNMonitor:
    """Monitors VPN connection and implements kill-switch behavior.
    
    When enabled, auto-pauses downloads if VPN drops and resumes
    on reconnection.
    """

    def __init__(
        self,
        provider: VPNProvider,
        on_vpn_drop: Callable[[], None] | None = None,
        on_vpn_restore: Callable[[], None] | None = None,
        poll_interval: float = 5.0,
    ):
        self.provider = provider
        self.on_vpn_drop = on_vpn_drop
        self.on_vpn_restore = on_vpn_restore
        self.poll_interval = poll_interval
        self.kill_switch_enabled = False
        self._was_connected = False
        self._running = False
        self._task: asyncio.Task | None = None

    async def start(self) -> None:
        self._running = True
        status = await self.provider.get_status()
        self._was_connected = status.is_connected
        self._task = asyncio.create_task(self._monitor_loop())

    async def stop(self) -> None:
        self._running = False
        if self._task:
            self._task.cancel()

    async def _monitor_loop(self) -> None:
        while self._running:
            try:
                await asyncio.sleep(self.poll_interval)
                if not self.kill_switch_enabled:
                    continue

                status = await self.provider.get_status()
                is_connected = status.is_connected

                if self._was_connected and not is_connected:
                    logger.warning("VPN connection dropped — kill-switch activated")
                    if self.on_vpn_drop:
                        self.on_vpn_drop()

                elif not self._was_connected and is_connected:
                    logger.info("VPN connection restored — resuming downloads")
                    if self.on_vpn_restore:
                        self.on_vpn_restore()

                self._was_connected = is_connected

            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.debug(f"VPN monitor error: {e}")
