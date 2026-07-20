"""Dedicated Proton VPN Manager supporting official Proton VPN CLI v3+."""

from __future__ import annotations

import asyncio
import logging
import os
import re
import shutil
import subprocess
from dataclasses import dataclass

logger = logging.getLogger(__name__)


@dataclass
class ProtonServer:
    """A Proton VPN server target."""
    label: str
    code: str
    flag: str  # "fastest", "country", "p2p", "securecore", "tor", "random"


@dataclass
class VpnStatus:
    """Current Proton VPN connection status."""
    connected: bool
    details: str = "Disconnected"
    server: str = ""


class ProtonVpnManager:
    """Manager for official Proton VPN CLI interactions."""

    SERVERS = [
        ProtonServer("Fastest Free Server (Auto)", "fastest", "fastest"),
        ProtonServer("Germany (DE)", "DE", "country"),
        ProtonServer("United States (US)", "US", "country"),
        ProtonServer("Netherlands (NL)", "NL", "country"),
        ProtonServer("Japan (JP)", "JP", "country"),
        ProtonServer("Switzerland (CH)", "CH", "country"),
        ProtonServer("United Kingdom (GB)", "GB", "country"),
        ProtonServer("Canada (CA)", "CA", "country"),
        ProtonServer("France (FR)", "FR", "country"),
        ProtonServer("Sweden (SE)", "SE", "country"),
        ProtonServer("P2P Optimized", "p2p", "p2p"),
        ProtonServer("Secure Core", "securecore", "securecore"),
        ProtonServer("Tor Over VPN", "tor", "tor"),
        ProtonServer("Random Server", "random", "random"),
    ]

    @classmethod
    def get_cli_path(cls) -> str | None:
        """Find protonvpn CLI executable."""
        return shutil.which("protonvpn") or shutil.which("protonvpn-cli")

    @classmethod
    def is_installed(cls) -> bool:
        """Check if Proton VPN CLI is available on the system."""
        return cls.get_cli_path() is not None

    @classmethod
    def _clean_output(cls, raw: str) -> str:
        """Strip deprecation warnings, sentry logs, and outdated server list banners from output."""
        lines = []
        for line in raw.split("\n"):
            stripped = line.strip()
            if not stripped:
                continue
            if "Eventlet" in stripped or "sentry_sdk" in stripped:
                continue
            if "Server list is outdated" in stripped or "updating... This may take a moment" in stripped:
                continue
            lines.append(stripped)
        return "\n".join(lines)

    @classmethod
    def get_status(cls) -> VpnStatus:
        """Check current Proton VPN connection status."""
        cli = cls.get_cli_path()
        if not cli:
            return VpnStatus(connected=False, details="Proton VPN CLI not installed")

        try:
            env = dict(os.environ, PYTHONWARNINGS="ignore")
            res = subprocess.run(
                [cli, "status"],
                capture_output=True,
                text=True,
                timeout=5,
                env=env,
            )
            clean_output = cls._clean_output((res.stdout or "") + "\n" + (res.stderr or ""))

            if "Connected" in clean_output and "Disconnected" not in clean_output:
                server_match = re.search(r"Server:\s*([^\n]+)", clean_output, re.IGNORECASE)
                server_name = server_match.group(1).strip() if server_match else "Connected"
                return VpnStatus(
                    connected=True,
                    details=clean_output,
                    server=server_name,
                )
            return VpnStatus(connected=False, details="Disconnected")
        except Exception as e:
            return VpnStatus(connected=False, details=f"Error checking status: {e}")

    @classmethod
    def check_account(cls) -> tuple[bool, str]:
        """Check if user is logged into Proton VPN."""
        cli = cls.get_cli_path()
        if not cli:
            return False, "Not Installed"

        try:
            env = dict(os.environ, PYTHONWARNINGS="ignore")
            res = subprocess.run([cli, "info"], capture_output=True, text=True, timeout=5, env=env)
            out = cls._clean_output((res.stdout or "") + "\n" + (res.stderr or ""))
            if "Account: 'None'" in out or "Not signed in" in out:
                return False, "Not Signed In"

            match = re.search(r"Account:\s*'([^']+)'", out)
            if match and match.group(1) != "None":
                return True, match.group(1)

            if "Plan:" in out or "Account:" in out:
                return True, "Logged In"
            return False, "Not Signed In"
        except Exception:
            return False, "Not Signed In"

    @classmethod
    async def async_get_status(cls) -> VpnStatus:
        """Check current Proton VPN connection status asynchronously off the UI thread."""
        return await asyncio.get_running_loop().run_in_executor(
            None, cls.get_status
        )

    @classmethod
    async def async_check_account(cls) -> tuple[bool, str]:
        """Check Proton VPN account status asynchronously off the UI thread."""
        return await asyncio.get_running_loop().run_in_executor(
            None, cls.check_account
        )

    @classmethod
    async def async_connect(cls, server: ProtonServer) -> tuple[bool, str]:
        """Connect to a Proton VPN server asynchronously off the UI thread."""
        return await asyncio.get_running_loop().run_in_executor(
            None, lambda: cls._sync_connect(server)
        )

    @classmethod
    async def async_disconnect(cls) -> tuple[bool, str]:
        """Disconnect from Proton VPN asynchronously."""
        return await asyncio.get_running_loop().run_in_executor(
            None, cls._sync_disconnect
        )

    @classmethod
    def _sync_connect(cls, server: ProtonServer) -> tuple[bool, str]:
        """Run synchronous protonvpn connect command."""
        cli = cls.get_cli_path()
        if not cli:
            return False, "Proton VPN CLI is not installed on this system."

        env = dict(os.environ, PYTHONWARNINGS="ignore")
        cmd = [cli, "connect"]

        if server.flag == "fastest":
            pass
        elif server.flag == "country":
            cmd.extend(["--country", server.code])
        elif server.flag == "p2p":
            cmd.append("--p2p")
        elif server.flag == "securecore":
            cmd.append("--securecore")
        elif server.flag == "tor":
            cmd.append("--tor")
        elif server.flag == "random":
            cmd.append("--random")

        try:
            res = subprocess.run(cmd, capture_output=True, text=True, timeout=30, env=env)
            output = cls._clean_output((res.stdout or "") + "\n" + (res.stderr or ""))

            if res.returncode == 0:
                return True, f"Proton VPN connected ({server.label})"

            # Handle Free Plan location restriction -> auto fallback to default connect
            if "not available on the free plan" in output.lower() or "free plan" in output.lower():
                logger.info("Country selection unavailable on Free plan; connecting to available free server...")
                fallback_res = subprocess.run([cli, "connect"], capture_output=True, text=True, timeout=30, env=env)
                fb_output = cls._clean_output((fallback_res.stdout or "") + "\n" + (fallback_res.stderr or ""))
                if fallback_res.returncode == 0:
                    return True, "Connected to available free server (Proton VPN Free)"
                return False, f"Location locked on Free plan. Connection failed: {fb_output}"

            if "invalid access token" in output.lower() or "signin" in output.lower():
                return False, "Proton VPN not signed in. Run 'protonvpn signin' in terminal."

            return False, output or f"Failed to connect ({server.label})"
        except subprocess.TimeoutExpired:
            return False, "Connection request timed out (30s)"
        except Exception as e:
            return False, str(e)

    @classmethod
    def _sync_disconnect(cls) -> tuple[bool, str]:
        """Run synchronous protonvpn disconnect command."""
        cli = cls.get_cli_path()
        if not cli:
            return False, "Proton VPN CLI is not installed."

        try:
            env = dict(os.environ, PYTHONWARNINGS="ignore")
            res = subprocess.run([cli, "disconnect"], capture_output=True, text=True, timeout=15, env=env)
            output = cls._clean_output((res.stdout or "") + "\n" + (res.stderr or ""))

            if res.returncode == 0:
                return True, "Proton VPN disconnected"
            return False, output or "Failed to disconnect Proton VPN"
        except Exception as e:
            return False, str(e)
