"""Proton VPN integration for Linux via official CLI.

Orchestrates the user's already-installed, already-authenticated
protonvpn CLI. Does not reimplement authentication or protocols.
"""

from __future__ import annotations

import asyncio
import logging
import platform
import shutil
import subprocess

from ytui.vpn.base import VPNProvider, VPNState, VPNStatus

logger = logging.getLogger(__name__)


class ProtonVPNLinux(VPNProvider):
    """Proton VPN provider using the official Linux CLI."""

    def __init__(self):
        self._cli_path = shutil.which("protonvpn-cli") or shutil.which("protonvpn")
        # Verify available flags at init rather than assuming syntax
        self._verified_flags: dict[str, bool] = {}

    def is_available(self) -> bool:
        return platform.system() == "Linux" and self._cli_path is not None

    async def _run_cli(self, *args: str) -> tuple[int, str, str]:
        """Run a protonvpn CLI command."""
        if not self._cli_path:
            return 1, "", "protonvpn CLI not found"
        
        cmd = [self._cli_path, *args]
        proc = None
        try:
            proc = await asyncio.create_subprocess_exec(
                *cmd,
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
            return 1, "", "Command timed out"
        except Exception as e:
            return 1, "", str(e)

    async def connect(
        self, server: str | None = None, country: str | None = None
    ) -> VPNStatus:
        if country:
            # Flags are illustrative — verify against installed version's --help
            code, out, err = await self._run_cli("connect", "--cc", country)
        elif server:
            code, out, err = await self._run_cli("connect", server)
        else:
            code, out, err = await self._run_cli("connect", "--fastest")

        if code == 0:
            return await self.get_status()
        return VPNStatus(
            state=VPNState.ERROR,
            error_message=err or out or "Connection failed",
        )

    async def disconnect(self) -> VPNStatus:
        code, out, err = await self._run_cli("disconnect")
        return VPNStatus(state=VPNState.DISCONNECTED)

    async def get_status(self) -> VPNStatus:
        code, out, err = await self._run_cli("status")
        if code != 0:
            return VPNStatus(
                state=VPNState.DISCONNECTED,
                error_message=err or "Unable to get status",
            )
        return self._parse_status(out)

    async def list_servers(self) -> list[dict[str, str]]:
        # Not all CLI versions support listing servers
        return []

    def _parse_status(self, output: str) -> VPNStatus:
        """Parse protonvpn status output."""
        status = VPNStatus(state=VPNState.DISCONNECTED)
        lines = output.strip().lower().split("\n")

        for line in lines:
            line = line.strip()
            if "connected" in line and "not" not in line and "dis" not in line:
                status.state = VPNState.CONNECTED
            elif "disconnected" in line or "not connected" in line:
                status.state = VPNState.DISCONNECTED
                return status

            if "server:" in line or "server name:" in line:
                status.server = line.split(":", 1)[1].strip()
            elif "country:" in line:
                status.country = line.split(":", 1)[1].strip().upper()
            elif "city:" in line:
                status.city = line.split(":", 1)[1].strip().title()
            elif "ip:" in line or "server ip:" in line:
                status.ip = line.split(":", 1)[1].strip()
            elif "protocol:" in line:
                status.protocol = line.split(":", 1)[1].strip()

        return status
