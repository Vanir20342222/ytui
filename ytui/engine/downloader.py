"""yt-dlp download engine wrapper.

Provides an async-friendly interface to yt-dlp with structured progress
callbacks bridged to Textual's worker/message system.
"""

from __future__ import annotations

import logging
import os
import subprocess
import threading
from typing import Any, Callable

import yt_dlp

from ytui.config.settings import Settings
from ytui.constants import AUDIO_FORMAT_MAP, VIDEO_FORMAT_MAP
from ytui.engine.metadata import extract_video_info
from ytui.queue.models import QueueItem, VideoInfo
from ytui.utils.filesystem import ensure_directory
from ytui.utils.urls import normalize_url

logger = logging.getLogger(__name__)

_original_popen = subprocess.Popen
_thread_processes: dict[int, list[subprocess.Popen]] = {}
_thread_processes_lock = threading.Lock()

class TrackedPopen(_original_popen):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        tid = threading.get_ident()
        with _thread_processes_lock:
            procs = [p for p in _thread_processes.get(tid, []) if p.poll() is None]
            procs.append(self)
            _thread_processes[tid] = procs

subprocess.Popen = TrackedPopen


class DownloadEngine:
    """Wraps yt-dlp as a library for structured downloading.
    
    Uses import yt_dlp directly (not subprocess), giving us access to
    progress_hooks and postprocessor_hooks for byte-accurate progress.
    """

    def __init__(self, settings: Settings):
        self.settings = settings
        self._active_ydls: dict[str, yt_dlp.YoutubeDL] = {}
        self._cancelled_items: set[str] = set()
        self._active_threads: dict[str, int] = {}

    def _build_opts(
        self,
        item: QueueItem,
        progress_callback: Callable[[dict], None] | None = None,
        postprocessor_callback: Callable[[dict], None] | None = None,
    ) -> dict[str, Any]:
        """Build yt-dlp options dict for a queue item."""
        s = self.settings
        mode = item.download_mode or s.quality.download_mode
        quality = item.quality_override or (
            s.quality.audio_quality if mode == "audio" else s.quality.video_quality
        )

        # Format selection
        if mode == "audio":
            fmt = AUDIO_FORMAT_MAP.get(quality, "bestaudio/best")
        else:
            fmt = VIDEO_FORMAT_MAP.get(quality, "bestvideo+bestaudio/best")

        # Output template and directory
        if mode == "audio":
            out_dir = s.directories.audio_dir
        else:
            out_dir = s.directories.video_dir
        ensure_directory(out_dir)

        outtmpl = os.path.join(out_dir, s.metadata.filename_template)

        opts: dict[str, Any] = {
            "format": fmt,
            "outtmpl": outtmpl,
            "quiet": True,
            "no_warnings": True,
            "ignoreerrors": False,
            "noprogress": True,
            "progress_hooks": [],
            "postprocessor_hooks": [],
            "postprocessors": [],
            "retries": s.advanced.auto_retry_count,
            "fragment_retries": 10,
            "socket_timeout": 30,
        }

        # Video container
        if mode == "video":
            opts["merge_output_format"] = s.quality.video_container
            # Prefer high frame-rate streams when enabled
            if s.quality.prefer_hfr:
                opts["format_sort"] = ["res", "fps:desc", "codec"]

            # Auto-convert videos to the configured container via FFmpeg.
            # This forces re-muxing/re-encoding when the source isn't already
            # in the target container (e.g. WebM source -> MP4 output).
            if s.quality.auto_convert_video:
                target = s.quality.video_container
                if target == "mp4":
                    opts["postprocessors"].append({
                        "key": "FFmpegVideoConvertor",
                        "preferedformat": "mp4",
                    })
                elif target == "mkv":
                    opts["postprocessors"].append({
                        "key": "FFmpegVideoConvertor",
                        "preferedformat": "mkv",
                    })
                elif target == "webm":
                    opts["postprocessors"].append({
                        "key": "FFmpegVideoConvertor",
                        "preferedformat": "webm",
                    })

        # Audio extraction
        if mode == "audio":
            codec_map = {
                "mp3": ("mp3", "libmp3lame"),
                "m4a": ("m4a", "aac"),
                "flac": ("flac", "flac"),
                "opus": ("opus", "libopus"),
                "ogg": ("vorbis", "libvorbis"),
                "wav": ("wav", "pcm_s16le"),
                "aac": ("m4a", "aac"),
            }
            container = s.quality.audio_container
            preferred_codec = codec_map.get(container, (container, container))[0]
            audio_quality_map = {
                "best": "0", "lossless": "0",
                "320": "320", "256": "256",
                "192": "192", "128": "128",
            }
            opts["postprocessors"].append({
                "key": "FFmpegExtractAudio",
                "preferredcodec": preferred_codec,
                "preferredquality": audio_quality_map.get(quality, "0"),
            })

        # Metadata embedding
        if s.metadata.write_metadata:
            opts["postprocessors"].append({"key": "FFmpegMetadata"})

        # Thumbnail embedding
        if s.metadata.embed_thumbnail:
            opts["writethumbnail"] = True
            opts["postprocessors"].append({"key": "EmbedThumbnail"})

        # Subtitles
        if s.subtitles.enabled:
            opts["writesubtitles"] = True
            opts["subtitleslangs"] = s.subtitles.languages
            if s.subtitles.auto_translate:
                opts["writeautomaticsub"] = True
            if s.subtitles.embed:
                opts["postprocessors"].append({"key": "FFmpegEmbedSubtitle"})

        # SponsorBlock
        if s.advanced.sponsor_block:
            raw_cats = s.advanced.sponsor_block_categories or ["sponsor", "intro", "outro", "selfpromo"]
            if isinstance(raw_cats, str):
                cats = [c.strip() for c in raw_cats.split(",") if c.strip()]
            elif isinstance(raw_cats, (list, tuple, set)):
                cats = [str(c).strip() for c in raw_cats if str(c).strip()]
            else:
                cats = ["sponsor", "intro", "outro", "selfpromo"]

            action = (s.advanced.sponsor_block_action or "mark").lower()

            sb_pp = {
                "key": "SponsorBlock",
                "categories": cats,
            }
            if s.advanced.sponsor_block_api:
                sb_pp["api"] = s.advanced.sponsor_block_api
                opts["sponsorblock_api"] = s.advanced.sponsor_block_api

            opts["postprocessors"].append(sb_pp)

            if action in ("skip", "remove"):
                opts["sponsorblock_remove"] = cats
                opts["postprocessors"].append({
                    "key": "ModifyChapters",
                    "remove_sponsor_segments": cats,
                })
            else:
                opts["sponsorblock_mark"] = cats

        # Bandwidth limiting (KB/s -> Bytes/s)
        try:
            per_limit = int(s.network.per_download_bandwidth_limit or 0)
        except (ValueError, TypeError):
            per_limit = 0

        try:
            global_limit = int(s.network.global_bandwidth_limit or 0)
        except (ValueError, TypeError):
            global_limit = 0

        limits = []
        if per_limit > 0:
            limits.append(per_limit * 1024)
        if global_limit > 0:
            try:
                concurrent = max(int(s.network.max_concurrent_downloads or 1), 1)
            except (ValueError, TypeError):
                concurrent = 1
            limits.append((global_limit * 1024) // concurrent)
        if limits:
            opts["ratelimit"] = int(min(limits))

        # Proxy
        if s.network.proxy_url:
            opts["proxy"] = s.network.proxy_url

        # Browser cookies & cookies file
        cookies_browser = s.network.cookies_from_browser or s.network.browser_cookies
        if cookies_browser:
            if isinstance(cookies_browser, tuple):
                opts["cookiesfrombrowser"] = cookies_browser
            elif isinstance(cookies_browser, str):
                opts["cookiesfrombrowser"] = tuple(cookies_browser.split(":"))

        cookies_file = s.network.cookies_file or s.network.cookie_file
        if cookies_file and os.path.exists(os.path.expanduser(cookies_file)):
            opts["cookiefile"] = os.path.expanduser(cookies_file)

        # Callbacks
        def _progress_wrapper(d: dict) -> None:
            if item.id in self._cancelled_items:
                raise yt_dlp.utils.DownloadCancelled("Download cancelled by user")
            if progress_callback:
                progress_callback(d)

        opts["progress_hooks"].append(_progress_wrapper)
        
        if postprocessor_callback:
            def _postprocessor_wrapper(d: dict) -> None:
                if item.id in self._cancelled_items:
                    raise yt_dlp.utils.DownloadCancelled("Download cancelled by user")
                postprocessor_callback(d)
            opts["postprocessor_hooks"].append(_postprocessor_wrapper)

        return opts

    def extract_info(
        self,
        url: str,
        flat: bool = False,
    ) -> dict[str, Any] | None:
        """Extract metadata for a URL without downloading.
        
        Args:
            url: The URL to extract info from.
            flat: If True, use extract_flat for playlists (fast mode).
        
        Returns:
            The info dict or None on error.
        """
        url = normalize_url(url)
        opts: dict[str, Any] = {
            "quiet": True,
            "no_warnings": True,
            "skip_download": True,
            "ignoreerrors": False,
        }
        if flat:
            opts["extract_flat"] = True

        # Proxy from settings
        if self.settings.network.proxy_url:
            opts["proxy"] = self.settings.network.proxy_url

        # Browser cookies & cookies file for auth
        cookies_browser = self.settings.network.cookies_from_browser or self.settings.network.browser_cookies
        if cookies_browser:
            if isinstance(cookies_browser, tuple):
                opts["cookiesfrombrowser"] = cookies_browser
            elif isinstance(cookies_browser, str):
                opts["cookiesfrombrowser"] = tuple(cookies_browser.split(":"))

        cookies_file = self.settings.network.cookies_file or self.settings.network.cookie_file
        if cookies_file and os.path.exists(os.path.expanduser(cookies_file)):
            opts["cookiefile"] = os.path.expanduser(cookies_file)

        try:
            with yt_dlp.YoutubeDL(opts) as ydl:
                info = ydl.extract_info(url, download=False)
                if info:
                    return ydl.sanitize_info(info)
        except Exception as e:
            import re
            msg = str(e)
            if "unavailable" in msg.lower():
                err_text = "Video unavailable"
            elif "private" in msg.lower():
                err_text = "This video is private"
            elif "age" in msg.lower() and "restrict" in msg.lower():
                err_text = "Age-restricted — browser cookies required"
            elif "copyright" in msg.lower():
                err_text = "Blocked due to copyright claim"
            elif "removed" in msg.lower():
                err_text = "Video has been removed"
            else:
                err_text = re.sub(r"^ERROR:\s*(\[\w+\]\s*[\w-]+:\s*)?", "", msg).strip()[:150]
            logger.error(f"Metadata extraction failed for {url}: {err_text}")
            raise RuntimeError(err_text or "Failed to fetch metadata")
        return None

    def download(
        self,
        item: QueueItem,
        progress_callback: Callable[[dict], None] | None = None,
        postprocessor_callback: Callable[[dict], None] | None = None,
    ) -> str | None:
        """Download a single item.
        
        Args:
            item: The queue item to download.
            progress_callback: Called with yt-dlp progress dict.
            postprocessor_callback: Called with yt-dlp postprocessor dict.
        
        Returns:
            The output file path on success, None on error.
        """
        url = normalize_url(item.url)
        # Clear any stale cancellation flag so resume/retry can proceed.
        tid = threading.get_ident()
        self._active_threads[item.id] = tid
        self._cancelled_items.discard(item.id)
        opts = self._build_opts(item, progress_callback, postprocessor_callback)

        try:
            with yt_dlp.YoutubeDL(opts) as ydl:
                self._active_ydls[item.id] = ydl
                info = ydl.extract_info(url, download=True)
                if info:
                    return ydl.prepare_filename(info)
        except yt_dlp.utils.DownloadError as e:
            error_msg = str(e)
            if "Requested format is not available" in error_msg:
                logger.warning(f"Requested format not available for {url}, attempting fallback format 'best'...")
                opts["format"] = "bestvideo+bestaudio/best" if item.download_mode == "video" else "bestaudio/best"
                try:
                    with yt_dlp.YoutubeDL(opts) as ydl:
                        self._active_ydls[item.id] = ydl
                        info = ydl.extract_info(url, download=True)
                        if info:
                            return ydl.prepare_filename(info)
                except Exception as fallback_err:
                    error_msg = str(fallback_err)

            # Provide user-friendly error messages
            if "Private video" in error_msg:
                item.error_message = "This video is private"
            elif "age" in error_msg.lower() and "restrict" in error_msg.lower():
                item.error_message = "Age-restricted — needs browser cookies for login"
            elif "unavailable" in error_msg.lower():
                item.error_message = "Video is unavailable"
            elif "copyright" in error_msg.lower():
                item.error_message = "Blocked due to copyright claim"
            elif "removed" in error_msg.lower():
                item.error_message = "Video has been removed"
            else:
                import re
                clean = re.sub(r"^ERROR:\s*(\[\w+\]\s*[\w-]+:\s*)?", "", error_msg).strip()
                item.error_message = clean[:150] or error_msg[:150]
            logger.error(f"Download failed for {url}: {error_msg}")
        except yt_dlp.utils.DownloadCancelled:
            # User-initiated cancel via engine.cancel() — propagate so the caller
            # can treat it as a pause/cancel rather than an error.
            logger.info(f"Download cancelled by user: {url}")
            raise
        except Exception as e:
            item.error_message = str(e)[:150]
            logger.error(f"Download error for {url}: {e}")
        finally:
            self._active_ydls.pop(item.id, None)
            self._active_threads.pop(item.id, None)
            with _thread_processes_lock:
                _thread_processes.pop(tid, None)
        return None

    def search(
        self,
        query: str,
        max_results: int = 10,
    ) -> list[VideoInfo]:
        """Search YouTube and return results as VideoInfo objects."""
        opts: dict[str, Any] = {
            "quiet": True,
            "no_warnings": True,
            "skip_download": True,
            "noplaylist": True,
        }

        if self.settings.network.proxy_url:
            opts["proxy"] = self.settings.network.proxy_url

        cookies_browser = self.settings.network.cookies_from_browser or self.settings.network.browser_cookies
        if cookies_browser:
            if isinstance(cookies_browser, tuple):
                opts["cookiesfrombrowser"] = cookies_browser
            elif isinstance(cookies_browser, str):
                opts["cookiesfrombrowser"] = tuple(cookies_browser.split(":"))

        cookies_file = self.settings.network.cookies_file or self.settings.network.cookie_file
        if cookies_file and os.path.exists(os.path.expanduser(cookies_file)):
            opts["cookiefile"] = os.path.expanduser(cookies_file)

        try:
            with yt_dlp.YoutubeDL(opts) as ydl:
                results = ydl.extract_info(
                    f"ytsearch{max_results}:{query}",
                    download=False,
                )
                if results and "entries" in results:
                    return [
                        extract_video_info(entry)
                        for entry in results["entries"]
                        if entry is not None
                    ]
        except Exception as e:
            logger.error(f"Search failed for '{query}': {e}")
        return []

    def cancel(self, item_id: str) -> None:
        """Cancel an active download."""
        self._cancelled_items.add(item_id)
        thread_id = self._active_threads.get(item_id)
        if thread_id is not None:
            with _thread_processes_lock:
                processes = _thread_processes.get(thread_id, [])
                for proc in processes:
                    if proc.poll() is None:
                        try:
                            proc.terminate()
                            proc.kill()
                        except Exception:
                            pass

    @staticmethod
    def get_version() -> str:
        """Get the installed yt-dlp version."""
        try:
            return yt_dlp.version.__version__
        except Exception:
            return "unknown"
