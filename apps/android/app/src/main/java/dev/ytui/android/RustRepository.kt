package dev.ytui.android

import android.graphics.Bitmap
import android.net.Uri
import android.util.LruCache
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import java.util.UUID
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.sync.withPermit
import org.json.JSONArray
import org.json.JSONObject

internal fun json(vararg fields: Pair<String, Any?>) =
    JSONObject().apply { fields.forEach { (key, value) -> put(key, value ?: JSONObject.NULL) } }

internal fun JSONObject.copyJson() = JSONObject(toString())

internal fun JSONArray.objects() = (0 until length()).mapNotNull { optJSONObject(it) }

internal class RpcError(
    val code: String,
    message: String,
    val retryMs: Long = 0,
    val retryable: Boolean = false,
) : Exception(message) {
    fun retryDelay(attempt: Int): Long? =
        if (retryable && attempt in 0..2) maxOf(retryMs, 2000L shl attempt) else null
}

/** Blocking JNI never runs on Android's main thread; cancellation reaches Rust immediately. */
internal object RustRpc {
    private val ids = AtomicLong(10_000)
    private val workers = ThreadPoolExecutor(8, 8, 0L, TimeUnit.SECONDS, ArrayBlockingQueue(64))

    suspend fun call(request: JSONObject): Any? = suspendCancellableCoroutine { continuation ->
        val id = ids.incrementAndGet()
        val payload = request.copyJson().put("request_id", id).toString()
        val future =
            try {
                workers.submit {
                    try {
                        val reply = JSONObject(NativeBridge.request(payload))
                        if (!reply.optBoolean("ok"))
                            throw RpcError(
                                reply.optString("code"),
                                reply.optString("error", "Request failed"),
                                reply.optLong("retry_after_ms"),
                                reply.optBoolean("retryable"),
                            )
                        if (continuation.isActive)
                            continuation.resume(
                                reply.opt("data").takeUnless { it == JSONObject.NULL }
                            )
                    } catch (error: Exception) {
                        if (continuation.isActive) continuation.resumeWithException(error)
                    }
                }
            } catch (busy: RejectedExecutionException) {
                continuation.resumeWithException(
                    RpcError("busy", "Too many requests. Please try again shortly.")
                )
                return@suspendCancellableCoroutine
            }
        continuation.invokeOnCancellation {
            NativeBridge.cancel(id)
            future.cancel(true)
        }
    }

    suspend fun objectCall(op: String, vararg fields: Pair<String, Any?>) =
        call(json("op" to op, *fields)) as? JSONObject ?: JSONObject()

    suspend fun arrayCall(op: String, vararg fields: Pair<String, Any?>) =
        call(json("op" to op, *fields)) as? JSONArray ?: JSONArray()
}

internal data class MediaCard(val value: JSONObject) {
    val title: String
        get() = value.optString("title")

    val creator: String
        get() = value.optString("creator")

    val url: String
        get() = value.optString("url")

    val kind: String
        get() = value.optString("kind", "video")

    val key: String
        get() = url.ifBlank { "$kind:${value.optString("id")}:$title" }

    val collection: Boolean
        get() = kind in setOf("playlist", "album", "artist", "channel")

    val audio: Boolean
        get() = kind == "song" || kind == "station"

    val live: Boolean
        get() = value.optBoolean("live")

    val musicSource: Boolean
        get() = Uri.parse(url).host.equals("music.youtube.com", ignoreCase = true)

    val collectionId: String
        get() {
            val id =
                value.optString("browse_id").takeUnless { it.isBlank() || it == "null" }
                    ?: value.optString("id")
            return if (kind == "playlist" && id.isNotBlank() && !id.startsWith("VL")) "VL$id"
            else id
        }
}

internal data class FeedState(
    val request: Any = "Home",
    val title: String = "For you",
    val cards: List<MediaCard> = emptyList(),
    val categories: List<JSONObject> = emptyList(),
    val continuation: String? = null,
    val loading: Boolean = false,
    val error: String? = null,
    val errorCode: String = "",
    val notice: String? = null,
    val generation: Long = 0,
    val scrollIndex: Int = 0,
    val failedRequest: Any? = null,
    val failedAppend: Boolean = false,
)

internal data class PlayerState(val value: JSONObject = JSONObject()) {
    val title: String
        get() = value.optString("title")

    val media: JSONObject?
        get() = value.optJSONObject("media")

    val playing: Boolean
        get() = value.optBoolean("playing")

    val position: Float
        get() =
            value.optDouble("position", 0.0).toFloat().takeIf { it.isFinite() }?.coerceAtLeast(0f)
                ?: 0f

    val duration: Float
        get() =
            value.optDouble("duration", 0.0).toFloat().takeIf { it.isFinite() }?.coerceAtLeast(0f)
                ?: 0f

    val video: Boolean
        get() = value.optBoolean("video")

    val preview: Boolean
        get() = value.optBoolean("preview")
}

internal object ArtworkCache {
    private val slots = Semaphore(8)
    private val memory =
        object : LruCache<String, Bitmap>(24 * 1024 * 1024) {
            override fun sizeOf(key: String, value: Bitmap) = value.allocationByteCount
        }

    suspend fun load(media: MediaCard, pixels: Int): Bitmap? {
        if (
            media.value.optString("thumbnail").isBlank() &&
                (media.value.optJSONArray("thumbnails")?.length() ?: 0) == 0
        )
            return null
        val size = pixels.coerceIn(32, 768)
        val key =
            "${media.key}:${media.value.optString("thumbnail")}:${media.value.optJSONArray("thumbnails")}:$size"
        memory.get(key)?.let {
            return it
        }
        return slots.withPermit {
            memory.get(key)
                ?: withContext(Dispatchers.IO) {
                    val data =
                        NativeBridge.thumbnail(media.value.toString(), size)
                            ?: return@withContext null
                    if (
                        data.size < 3 ||
                            data[0] !in 1..768 ||
                            data[1] !in 1..768 ||
                            data.size != data[0] * data[1] + 2
                    )
                        return@withContext null
                    Bitmap.createBitmap(data, 2, data[0], data[0], data[1], Bitmap.Config.ARGB_8888)
                        .also { memory.put(key, it) }
                }
        }
    }
}

internal class YtuiViewModel : ViewModel() {
    val tab = MutableStateFlow(1)
    val feeds = Array(4) { MutableStateFlow(FeedState()) }
    private val history = Array(4) { ArrayDeque<FeedState>() }
    private val feedJobs = arrayOfNulls<Job>(4)
    private var playbackJob: Job? = null
    private var seekTarget: Pair<String, Float>? = null
    private var seekDeadline = 0L
    private var accountEpoch = 0L
    private val settingsLock = Mutex()
    private val snapshotLock = Mutex()
    val mobile = MutableStateFlow(JSONObject())
    val config = MutableStateFlow(JSONObject())
    val palette = MutableStateFlow(JSONObject())
    val themes = MutableStateFlow<List<String>>(emptyList())
    val account = MutableStateFlow(JSONObject())
    val player = MutableStateFlow(PlayerState())
    val downloads = MutableStateFlow<List<JSONObject>>(emptyList())
    val offlinePlaylists = MutableStateFlow<List<JSONObject>>(emptyList())
    val queue = MutableStateFlow<List<MediaCard>>(emptyList())
    val recent = MutableStateFlow<List<MediaCard>>(emptyList())
    val status = MutableStateFlow("Ready")
    val actionError = MutableStateFlow<String?>(null)
    val ready = MutableStateFlow(false)
    val playerOpen = MutableStateFlow(false)
    val sharedQuery = MutableStateFlow("")
    private var initialized = false

    fun initialize(dark: Boolean) {
        if (initialized) return
        initialized = true
        viewModelScope.launch {
            runCatching {
                    mobile.value = withContext(Dispatchers.IO) { MobileSettings.snapshot() }
                    config.value = RustRpc.objectCall("settings")
                    palette.value = RustRpc.objectCall("theme", "dark" to dark)
                    themes.value =
                        RustRpc.arrayCall("themes").let { values ->
                            (0 until values.length()).map { values.getString(it) }
                        }
                    account.value = RustRpc.objectCall("account")
                    account.value
                        .optString("notice")
                        .takeUnless { it.isBlank() || it == "null" }
                        ?.let { status.value = it }
                }
                .onFailure { status.value = it.message ?: "Could not load settings" }
            ready.value = true
            if (!MediaBridge.offlineUi) navigate(1, "Home", remember = false)
        }
    }

    fun systemTheme(dark: Boolean) {
        task {
            settingsLock.withLock {
                if (config.value.optString("theme", "system") == "system")
                    palette.value = RustRpc.objectCall("theme", "dark" to dark)
            }
        }
    }

    suspend fun snapshot() =
        withContext(Dispatchers.Main.immediate) {
            snapshotLock.withLock {
                val value = withContext(Dispatchers.Default) { JSONObject(MediaBridge.snapshot()) }
                value.optJSONObject("player")?.let {
                    val pending = seekTarget
                    if (pending != null) {
                        val same = pending.first == it.optJSONObject("media")?.optString("url")
                        if (
                            !same ||
                                kotlin.math.abs(it.optDouble("position") - pending.second) < 1.25 ||
                                android.os.SystemClock.elapsedRealtime() >= seekDeadline
                        )
                            seekTarget = null
                        else it.put("position", pending.second)
                    }
                    player.value = PlayerState(it)
                }
                value.optJSONArray("downloads")?.let { downloads.value = it.objects() }
                value.optJSONArray("events")?.objects()?.forEach { event ->
                    val text = event.optString("text")
                    when (event.optString("kind")) {
                        "storage_updated" -> {
                            mobile.value = withContext(Dispatchers.IO) { MobileSettings.snapshot() }
                            OfflineStore.dirty = true
                        }
                        "show_player" -> playerOpen.value = true
                        "shared" -> sharedQuery.value = text
                        "error" -> {
                            status.value = text
                            actionError.value = text
                        }
                        "theme_file" ->
                            task {
                                settingsLock.withLock {
                                    palette.value =
                                        RustRpc.objectCall("import_theme", "text" to text)
                                    config.value = RustRpc.objectCall("settings")
                                    status.value = "Palette applied"
                                }
                            }
                        "cookie_file" -> {
                            val intent = event.optLong("account_intent", -1)
                            if (intent == NativeBridge.accountIntent())
                                acceptAccount("import_cookie_file", intent, "text" to text)
                        }
                        "cookies" -> changeAccount("import_cookies", "header" to text)
                        "account_updated" -> refreshAccount()
                        "history_updated",
                        "history_cleared" -> loadHistory()
                        "lyrics_file" ->
                            if (event.optString("request_token") == lyricToken)
                                importedLyrics.value = text
                    }
                }
            }
        }

    fun chooseTab(index: Int) {
        tab.value = index
        if (index > 0 && feeds[index].value.generation == 0L && !MediaBridge.offlineUi)
            navigate(index, if (index == 3) "Cliamp" else "Home", remember = false)
    }

    fun rememberScroll(index: Int, position: Int) {
        feeds[index].value = feeds[index].value.copy(scrollIndex = position)
    }

    fun navigate(
        index: Int = tab.value,
        request: Any,
        append: Boolean = false,
        remember: Boolean = true,
        retry: Int = 0,
        autoPlay: Boolean = false,
    ) {
        if (index !in 1..3) return
        val previous = feeds[index].value
        if (append && (previous.loading || previous.continuation == null)) return
        feedJobs[index]?.cancel()
        if (!append && remember && previous.generation > 0) {
            history[index].addLast(previous.copy(loading = false))
            while (
                history[index].size > 12 ||
                    history[index].sumOf {
                        it.cards.sumOf { card -> card.value.toString().length * 2 }
                    } > 2 * 1024 * 1024
            ) history[index].removeFirst()
        }
        val generation = previous.generation + 1
        val loading =
            if (append) previous.copy(loading = true, error = null, generation = generation)
            else FeedState(request = request, loading = true, generation = generation)
        feeds[index].value = loading
        feedJobs[index] =
            viewModelScope.launch {
                try {
                    val page =
                        RustRpc.objectCall(
                            "browse",
                            "tab" to index,
                            "request" to request,
                            "offset" to if (append) previous.cards.size else 0,
                        )
                    if (feeds[index].value.generation != generation) return@launch
                    val incoming =
                        page.optJSONArray("items")?.objects()?.map(::MediaCard) ?: emptyList()
                    val cards =
                        (if (append) previous.cards + incoming else incoming)
                            .distinctBy { it.key }
                            .takeLast(2000)
                    feeds[index].value =
                        loading.copy(
                            request = if (append) previous.request else request,
                            title = page.optString("title", previous.title),
                            cards = cards,
                            categories =
                                if (append) previous.categories
                                else page.optJSONArray("categories")?.objects() ?: emptyList(),
                            continuation =
                                page.optString("continuation").takeUnless {
                                    it.isBlank() || it == "null"
                                },
                            notice =
                                page.optString("notice").takeUnless {
                                    it.isBlank() || it == "null"
                                },
                            loading = false,
                            scrollIndex = feeds[index].value.scrollIndex,
                        )
                    if (autoPlay && tab.value == index)
                        incoming.firstOrNull { !it.collection && !it.live }?.let { play(it) }
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (error: Exception) {
                    if (feeds[index].value.generation != generation) return@launch
                    feeds[index].value =
                        loading.copy(
                            loading = false,
                            error = error.message,
                            errorCode = (error as? RpcError)?.code ?: "",
                            failedRequest = request,
                            failedAppend = append,
                            scrollIndex = feeds[index].value.scrollIndex,
                        )
                    val retryDelay = (error as? RpcError)?.retryDelay(retry)
                    if (retryDelay != null) {
                        delay(retryDelay)
                        if (feeds[index].value.generation == generation)
                            navigate(
                                index,
                                request,
                                append,
                                remember = false,
                                retry = retry + 1,
                                autoPlay = autoPlay,
                            )
                    }
                }
            }
    }

    fun more() {
        feeds[tab.value].value.continuation?.let {
            navigate(request = json("Continue" to it), append = true)
        }
    }

    fun retryFeed() {
        val feed = feeds[tab.value].value
        navigate(
            request = feed.failedRequest ?: feed.request,
            append = feed.failedAppend,
            remember = false,
        )
    }

    fun back(): Boolean {
        if (history[tab.value].isEmpty()) return false
        val index = tab.value
        feedJobs[index]?.cancel()
        feeds[index].value =
            history[index]
                .removeLast()
                .copy(generation = feeds[index].value.generation + 1, loading = false)
        return true
    }

    fun search(raw: String) {
        val query = raw.trim()
        if (query.isBlank()) return
        if (query.startsWith("https://") || query.startsWith("http://")) {
            runCatching { linked(query) }
                .onSuccess { play(it) }
                .onFailure { status.value = "Enter a valid HTTP(S) video or playlist URL" }
        } else navigate(request = json((if (tab.value == 3) "RadioSearch" else "Search") to query))
    }

    fun linked(raw: String): MediaCard {
        val uri = java.net.URI(raw)
        require(uri.scheme in listOf("http", "https") && uri.host != null && uri.userInfo == null)
        val parsed = Uri.parse(raw)
        val host = uri.host.lowercase(java.util.Locale.ROOT)
        val youtube = host == "youtube.com" || host.endsWith(".youtube.com") || host == "youtu.be"
        val playlist =
            youtube &&
                parsed.path?.trimEnd('/') == "/playlist" &&
                !parsed.getQueryParameter("list").isNullOrBlank()
        return MediaCard(
            json(
                "url" to raw,
                "title" to raw,
                "id" to if (playlist) parsed.getQueryParameter("list") else "",
                "kind" to
                    if (playlist) "playlist"
                    else if (tab.value == 3) "station"
                    else if (tab.value == 2 || host == "music.youtube.com") "song" else "video",
                "live" to (tab.value == 3),
            )
        )
    }

    fun tracks(media: MediaCard) {
        media.value
            .optString("tracks_url")
            .takeUnless { it.isBlank() || it == "null" }
            ?.let { navigate(request = json("CliampTracks" to it)) }
    }

    private fun context(media: MediaCard): JSONObject? {
        if (media.live || media.collection) return null
        val feed = feeds[tab.value].value
        val request = feed.request as? JSONObject
        val finite =
            request != null && listOf("Collection", "Details", "CliampTracks").any(request::has)
        val id =
            media.value.optString("id").takeIf { it.matches(Regex("[A-Za-z0-9_-]{11}")) }
                ?: Uri.parse(media.url)
                    .let { uri ->
                        uri.getQueryParameter("v")
                            ?: if (uri.host == "youtu.be") uri.lastPathSegment else null
                    }
                    ?.takeIf { it.matches(Regex("[A-Za-z0-9_-]{11}")) }
        if (!finite && (!mobile.value.optBoolean("autoplay_related", true) || id == null))
            return null
        return json(
                "op" to "prepare_context",
                "context_id" to UUID.randomUUID().toString(),
                "request" to if (finite) request else json("Related" to id),
                "source_url" to media.url,
                "source_id" to id,
                "music" to (tab.value == 2 || media.musicSource),
            )
            .apply { if (finite) put("items", JSONArray(feed.cards.take(500).map { it.value })) }
    }

    fun playCollection(media: MediaCard) {
        if (media.collection)
            navigate(request = json("Collection" to media.collectionId), autoPlay = true)
    }

    fun play(
        media: MediaCard,
        preview: Boolean = false,
        download: Boolean = false,
        resume: Boolean = false,
        queued: Boolean = false,
    ) {
        val quality = config.value.optJSONObject("download")?.optString("quality", "1080") ?: "1080"
        val audio =
            media.audio ||
                (media.collection && (media.musicSource || tab.value == 2)) ||
                download && config.value.optJSONObject("download")?.optBoolean("audio") == true
        if (download && media.live) {
            status.value = "Live recording is not available"
            return
        }
        if (media.collection) {
            if (download)
                effect(
                    "download_playlist",
                    "request" to json("Collection" to media.collectionId),
                    "music" to (tab.value == 2 || media.musicSource),
                    "audio" to audio,
                    "quality" to quality,
                )
            else navigate(request = json("Collection" to media.collectionId))
            return
        }
        val context = if (!preview && !download && !queued) context(media) else null
        val epoch = accountEpoch
        val network = NetworkSettings.revision.get()
        if (!download) playbackJob?.cancel()
        val job = task {
            actionError.value = null
            status.value = if (download) "Preparing download…" else "Resolving playback…"
            val local =
                if (!download && !preview && mobile.value.optBoolean("prefer_offline", true))
                    withContext(Dispatchers.IO) { OfflineStore.find(media.value, audio, quality) }
                else null
            val stream =
                local?.optJSONObject("stream")
                    ?: RustRpc.objectCall(
                        "resolve",
                        "media" to media.value,
                        "audio" to audio,
                        "quality" to quality,
                    )
            if (epoch != accountEpoch || network != NetworkSettings.revision.get()) return@task
            val item = media.value.copyJson()
            PlaybackService.mergeMetadata(item, stream)
            val request =
                (local ?: JSONObject())
                    .apply { put("media", item) }
                    .apply {
                        put("stream", stream)
                        put("audio", audio)
                        put("quality", quality)
                        put("preview", preview)
                        put("resume", resume)
                        put("queued", queued)
                        put("context_id", context?.optString("context_id"))
                        put("context", context)
                    }
            /* request retains the cache-only source marker when saved offline. */
            effect(if (download) "download" else "play", "request" to request)
            if (
                !download &&
                    (!audio || config.value.optString("music_player_style").endsWith("-fullscreen"))
            )
                playerOpen.value = true
            status.value = if (download) "Download queued" else "Starting playback"
        }
        if (!download) playbackJob = job
    }

    fun effect(op: String, vararg fields: Pair<String, Any?>) {
        MediaBridge.execute(json("op" to op, *fields).toString())
    }

    fun command(op: String, vararg fields: Pair<String, Any?>) {
        if (op == "seek") {
            val seconds =
                (fields.firstOrNull { it.first == "seconds" }?.second as? Number)?.toFloat()
            if (seconds == null || !seconds.isFinite() || player.value.duration <= 0) return
            val target = seconds.coerceIn(0f, player.value.duration)
            seekTarget = (player.value.media?.optString("url") ?: "") to target
            seekDeadline = android.os.SystemClock.elapsedRealtime() + 2000
            player.value = PlayerState(player.value.value.copyJson().put("position", target))
        }
        if (op == "pause")
            player.value =
                PlayerState(
                    player.value.value
                        .copyJson()
                        .put("playing", !player.value.playing)
                        .put("paused", player.value.playing)
                )
        if (op == "volume" || op == "speed") {
            val value = fields.firstOrNull { it.first == "value" }?.second as? Number
            if (value != null && value.toDouble().isFinite()) {
                player.value = PlayerState(player.value.value.copyJson().put(op, value))
            }
        }
        effect(op, *fields)
    }

    fun promotePreview() {
        val media = player.value.media?.let(::MediaCard) ?: return
        val context = context(media)
        effect(
            "preview_play_full",
            "context" to context,
            "context_id" to context?.optString("context_id"),
        )
    }

    fun save(media: MediaCard) = task {
        val result = RustRpc.objectCall("save_media", "media" to media.value)
        status.value =
            if (result.optBoolean("saved")) "Saved locally" else "Removed from saved items"
        if (feeds[tab.value].value.request == "Favorites")
            navigate(request = "Favorites", remember = false)
    }

    fun enqueue(media: MediaCard) = task {
        RustRpc.call(
            json(
                "op" to if (media.collection) "enqueue_collection" else "enqueue",
                "media" to media.value,
                "request" to json("Collection" to media.collectionId),
                "music" to (tab.value == 2 || media.musicSource),
            )
        )
        loadQueue()
        effect("queue_changed")
        status.value = "Added to queue"
    }

    fun loadOfflinePlaylists() = task {
        offlinePlaylists.value = RustRpc.arrayCall("offline_playlists").objects()
    }

    fun createOfflinePlaylist(title: String) = task {
        offlinePlaylists.value =
            RustRpc.arrayCall("offline_playlist_create", "title" to title).objects()
    }

    fun updateOfflinePlaylist(id: String, title: String? = null, items: List<JSONObject>? = null) =
        task {
            val request = json("op" to "offline_playlist_update", "id" to id)
            title?.let { request.put("title", it) }
            items?.let { request.put("items", JSONArray(it)) }
            offlinePlaylists.value = (RustRpc.call(request) as? JSONArray)?.objects() ?: emptyList()
        }

    fun deleteOfflinePlaylist(id: String) = task {
        offlinePlaylists.value = RustRpc.arrayCall("offline_playlist_delete", "id" to id).objects()
    }

    fun playOffline(item: JSONObject) {
        effect("offline", "id" to item.optString("id"))
        if (!item.optBoolean("audio")) playerOpen.value = true
    }

    fun playOfflinePlaylist(playlist: JSONObject, start: Int = 0) {
        val items = playlist.optJSONArray("items")?.objects() ?: return
        val original = items.getOrNull(start) ?: return
        val ids =
            downloads.value
                .filter { it.optString("state") == "Complete" }
                .map { it.optString("id") }
                .toSet()
        val available = items.filter { it.optString("offline_id") in ids }
        val selected =
            if (start == 0) available.firstOrNull()
            else original.takeIf { it.optString("offline_id") in ids }
        if (selected == null) {
            actionError.value =
                "No saved copy is available. Save the media again or reconnect its folder."
            return
        }
        val token = UUID.randomUUID().toString()
        val context =
            json(
                "op" to "offline_context",
                "context_id" to token,
                "source_url" to selected.optString("url"),
                "items" to JSONArray(available),
            )
        effect(
            "offline",
            "id" to selected.optString("offline_id"),
            "context_id" to token,
            "context" to context,
        )
        if (!PlaybackPolicy.audioOnly(selected.optString("kind"))) playerOpen.value = true
    }

    fun loadQueue() = task { queue.value = RustRpc.arrayCall("queue").objects().map(::MediaCard) }

    fun moveQueue(index: Int, to: Int, media: MediaCard) = task {
        val result =
            RustRpc.objectCall("queue_move", "from" to index, "to" to to, "url" to media.url)
        queue.value = result.optJSONArray("items")?.objects()?.map(::MediaCard) ?: emptyList()
        effect("queue_changed")
    }

    fun removeQueue(index: Int, media: MediaCard) = task {
        val result = RustRpc.objectCall("queue_remove", "index" to index, "url" to media.url)
        queue.value = result.optJSONArray("items")?.objects()?.map(::MediaCard) ?: emptyList()
        effect("queue_changed")
    }

    fun loadHistory() = task {
        recent.value = RustRpc.arrayCall("history_list").objects().map(::MediaCard)
    }

    fun updateConfig(dark: Boolean, edit: (JSONObject) -> Unit) = task {
        settingsLock.withLock {
            val draft = config.value.copyJson()
            edit(draft)
            RustRpc.objectCall("save_settings", "config" to draft)
            val appliedPalette = RustRpc.objectCall("theme", "dark" to dark)
            palette.value = appliedPalette
            config.value = draft
            effect("settings", "config" to draft)
            status.value = "Settings applied"
            actionError.value = null
        }
    }

    fun updateMobile(key: String, value: Any) = task {
        settingsLock.withLock {
            mobile.value =
                withContext(Dispatchers.IO) {
                    MobileSettings.save(json(key to value))
                    MobileSettings.snapshot()
                }
        }
    }

    private fun invalidateAccount() {
        PlaybackCache.cancel()
        accountEpoch++
        playbackJob?.cancel()
        for (index in 1..2) {
            feedJobs[index]?.cancel()
            history[index].clear()
            feeds[index].value = FeedState(generation = feeds[index].value.generation + 1)
        }
    }

    fun changeAccount(op: String, vararg fields: Pair<String, Any?>) {
        acceptAccount(op, NativeBridge.beginAccountChange(), *fields)
    }

    private fun acceptAccount(op: String, intent: Long, vararg fields: Pair<String, Any?>) {
        invalidateAccount()
        val epoch = accountEpoch
        if (op == "disconnect") effect("disconnect")
        task {
            val result = RustRpc.objectCall(op, "account_intent" to intent, *fields)
            if (epoch != accountEpoch) return@task
            account.value = result
            status.value =
                if (account.value.optBoolean("connected")) "Account connected"
                else
                    account.value.optString("notice").takeUnless { it.isBlank() || it == "null" }
                        ?: "Disconnected"
            if (!MediaBridge.offlineUi && tab.value in 1..2)
                navigate(request = "Home", remember = false)
        }
    }

    private fun refreshAccount() {
        invalidateAccount()
        val epoch = accountEpoch
        task {
            val result = RustRpc.objectCall("account")
            if (epoch != accountEpoch) return@task
            account.value = result
            if (!MediaBridge.offlineUi && tab.value in 1..2)
                navigate(request = "Home", remember = false)
        }
    }

    val lyricData = MutableStateFlow(JSONObject())
    val lyricVersions = MutableStateFlow<List<JSONObject>>(emptyList())
    val importedLyrics = MutableStateFlow<String?>(null)
    private var lyricToken = ""
    private var lyricRevision = 0L

    fun lyricLoad(media: JSONObject) {
        lyricToken = UUID.randomUUID().toString()
        lyricData.value = JSONObject()
        lyricVersions.value = emptyList()
        importedLyrics.value = null
        val token = lyricToken
        val revision = ++lyricRevision
        task {
            val result = RustRpc.objectCall("lyrics_get", "url" to media.optString("url"))
            if (token == lyricToken && revision == lyricRevision) lyricData.value = result
        }
    }

    fun lyricImport() {
        effect("pick", "kind" to "lyrics", "request_token" to lyricToken)
    }

    fun lyricClose() {
        lyricToken = ""
        lyricRevision++
        importedLyrics.value = null
    }

    fun lyricLookup(media: JSONObject, title: String, artist: String) = task {
        val token = lyricToken
        val revision = ++lyricRevision
        val result =
            RustRpc.arrayCall("lyrics", "media" to media, "title" to title, "artist" to artist)
                .objects()
        if (token == lyricToken && revision == lyricRevision) lyricVersions.value = result
    }

    fun lyricSave(media: JSONObject, document: JSONObject) = task {
        val token = lyricToken
        val revision = ++lyricRevision
        val result =
            RustRpc.objectCall(
                "lyrics_save",
                "url" to media.optString("url"),
                "document" to document,
            )
        if (token == lyricToken && revision == lyricRevision) lyricData.value = result
        status.value = "Lyrics saved locally"
    }

    fun lyricRemove(media: JSONObject) = task {
        val token = lyricToken
        val revision = ++lyricRevision
        RustRpc.objectCall("lyrics_remove", "url" to media.optString("url"))
        if (token == lyricToken && revision == lyricRevision) lyricData.value = JSONObject()
        status.value = "Saved lyrics removed"
    }

    private fun task(block: suspend CoroutineScope.() -> Unit): Job =
        viewModelScope.launch {
            try {
                block()
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                status.value = error.message ?: "Operation failed"
                actionError.value = status.value
            }
        }

    override fun onCleared() {
        playbackJob?.cancel()
        feedJobs.forEach { it?.cancel() }
        super.onCleared()
    }
}
