"""WireGuard-based VPN for Windows/macOS.

Uses Proton VPN's WireGuard config exports driven through
OS-native WireGuard tooling. No proprietary reimplementation.
"""

from __future__ import annotations

import asyncio
import logging
import platform
import shutil
from pathlib import Path

from ytui.config.settings import _config_dir
from ytui.vpn.base import VPNProvider, VPNState, VPNStatus

logger = logging.getLogger(__name__)


class WireGuardVPN(VPNProvider):
    """WireGuard VPN provider for Windows/macOS."""

    def __init__(self):
        self._configs_dir = _config_dir() / "wireguard"
        self._configs_dir.mkdir(parents=True, exist_ok=True)
        self._active_config: str | None = None
        self._wg_quick = shutil.which("wg-quick")
        self._wg = shutil.which("wg")

    def is_available(self) -> bool:
        system = platform.system()
        if system == "Linux":
            return False  # Use ProtonVPN CLI on Linux
        return self._wg_quick is not None or self._wg is not None

    def get_configs(self) -> list[str]:
        """List available WireGuard config files."""
        return sorted(
            p.stem for p in self._configs_dir.glob("*.conf")
        )

    async def connect(
        self, server: str | None = None, country: str | None = None
    ) -> VPNStatus:
        config_name = server or (self.get_configs()[0] if self.get_configs() else None)
        if not config_name:
            return VPNStatus(
                state=VPNState.ERROR,
                error_message="No WireGuard configs available. Add .conf files to the VPN panel.",
            )

        safe_name = Path(config_name).name
        config_path = (self._configs_dir / f"{safe_name}.json" if not safe_name.endswith(".conf") else self._configs_dir / safe_name)
        if not config_path.exists():
            config_path = self._configs_dir / f"{safe_name}.conf"
        if not config_path.exists():
            config_path = self._configs_dir / safe_name
        try:
            resolved_config = config_path.resolve()
            resolved_dir = self._configs_dir.resolve()
            if not str(resolved_config).startswith(str(resolved_dir)):
                return VPNStatus(
                    state=VPNState.ERROR,
                    error_message=f"Invalid config name '{config_name}'",
                )
        except Exception:
            return VPNStatus(
                state=VPNState.ERROR,
                error_message=f"Invalid config name '{config_name}'",
            )
        if not config_path.exists():
            return VPNStatus(
                state=VPNState.ERROR,
                error_message=f"Config '{config_name}' not found",
            )

        # Connect using wg-quick
        system = platform.system()
        if system == "Darwin":
            code, out, err = await self._run_cmd(
                "wg-quick", "up", str(config_path)
            )
        elif system == "Windows":
            # On Windows, use wireguard.exe /installtunnelservice
            code, out, err = await self._run_cmd(
                "wireguard.exe", "/installtunnelservice", str(config_path)
            )
        else:
            code, out, err = await self._run_cmd(
                "wg-quick", "up", str(config_path)
            )

        if code == 0:
            self._active_config = config_name
            return VPNStatus(
                state=VPNState.CONNECTED,
                server=config_name,
            )
        return VPNStatus(
            state=VPNState.ERROR,
            error_message=err or out or "Connection failed",
        )

    async def disconnect(self) -> VPNStatus:
        if not self._active_config:
            return VPNStatus(state=VPNState.DISCONNECTED)

        config_path = self._configs_dir / f"{self._active_config}.conf"
        system = platform.system()

        if system == "Darwin":
            await self._run_cmd("wg-quick", "down", str(config_path))
        elif system == "Windows":
            await self._run_cmd(
                "wireguard.exe", "/uninstalltunnelservice", self._active_config
            )
        else:
            await self._run_cmd("wg-quick", "down", str(config_path))

        self._active_config = None
        return VPNStatus(state=VPNState.DISCONNECTED)

    async def get_status(self) -> VPNStatus:
        if self._active_config:
            return VPNStatus(
                state=VPNState.CONNECTED,
                server=self._active_config,
            )
        return VPNStatus(state=VPNState.DISCONNECTED)

    async def list_servers(self) -> list[dict[str, str]]:
        return [
            {"name": name, "type": "wireguard"}
            for name in self.get_configs()
        ]

    async def _run_cmd(self, *args: str) -> tuple[int, str, str]:
        proc = None
        try:
            proc = await asyncio.create_subprocess_exec(
                *args,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
            stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=30)
            return (
                proc.returncode or 0,
                stdout.decode(errors="replace"),
                stderr.decode(errors="replace"),
            )
        except asyncio.TimeoutError:
            if proc:
                try:
                    proc.kill()
                    await proc.wait()
                except Exception:
                    pass
            return 1, "", "Command timed out after 30 seconds"
        except Exception as e:
            return 1, "", str(e)
