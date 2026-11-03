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
    private val playbackWorkers =
        ThreadPoolExecutor(2, 2, 0L, TimeUnit.SECONDS, ArrayBlockingQueue(8))

    suspend fun call(request: JSONObject): Any? = suspendCancellableCoroutine { continuation ->
        val id = ids.incrementAndGet()
        val payload = request.copyJson().put("request_id", id).toString()
        val queuedAt = android.os.SystemClock.elapsedRealtime()
        val timing = request.optLong("startup_token")
        val pool =
            if (request.optString("op") == "resolve" && !request.optBoolean("speculative"))
                playbackWorkers
            else workers
        val future =
            try {
                pool.submit {
                    StartupTimings.stage(
                        timing,
                        "rpc_queue",
                        android.os.SystemClock.elapsedRealtime() - queuedAt,
                    )
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
                        if (continuation.isActive) {
                            StartupTimings.ended(
                                timing,
                                "failed",
                                android.os.SystemClock.elapsedRealtime(),
                            )
                            continuation.resumeWithException(error)
                        }
                    }
                }
            } catch (busy: RejectedExecutionException) {
                continuation.resumeWithException(
                    RpcError("busy", "Too many requests. Please try again shortly.")
                )
                return@suspendCancellableCoroutine
            }
        continuation.invokeOnCancellation {
            StartupTimings.ended(timing, "cancelled", android.os.SystemClock.elapsedRealtime())
            NativeBridge.cancel(id)
            future.cancel(true)
            // Cancelled queued requests must not consume admission capacity while
            // older native calls finish; the latest tap still needs a slot.
            pool.purge()
        }
    }

    suspend fun objectCall(op: String, vararg fields: Pair<String, Any?>) =
        call(json("op" to op, *fields)) as? JSONObject ?: JSONObject()

    suspend fun arrayCall(op: String, vararg fields: Pair<String, Any?>) =
        call(json("op" to op, *fields)) as? JSONArray ?: JSONArray()
}

/** What plays after the current item: queue, then the active playback context. */
internal data class UpNextState(
    val context: List<MediaCard> = emptyList(),
    val contextId: String = "",
    /** Related videos for the current one, rather than the rest of a playlist. */
    val related: Boolean = false,
    /** False for recommendations shown while related autoplay is off. */
    val autoplay: Boolean = true,
    val more: Boolean = false,
    val loading: Boolean = false,
)

/** Treated as immutable: media JSON is copied (`copyJson`) before any change. */
@androidx.compose.runtime.Immutable
internal data class MediaCard(val value: JSONObject) {
    /** Thumbnail identity, computed once instead of re-serialising arrays per frame. */
    val artworkSource: String by lazy {
        "${value.optString("thumbnail")}:${value.optJSONArray("thumbnails")}"
    }

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

@androidx.compose.runtime.Immutable
internal data class PreparingVideo(val token: Long, val media: MediaCard, val restoreOpen: Boolean)

@androidx.compose.runtime.Immutable
internal data class PlayerState(
    val value: JSONObject = JSONObject(),
    val preparing: PreparingVideo? = null,
) {
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
        get() = preparing != null || value.optBoolean("video")

    val preview: Boolean
        get() = value.optBoolean("preview")
}

internal object ArtworkCache {
    private val slots = Semaphore(8)
    private val memory =
        object : LruCache<String, Bitmap>(24 * 1024 * 1024) {
            override fun sizeOf(key: String, value: Bitmap) = value.allocationByteCount
        }

    private val decodeSizes = intArrayOf(96, 160, 240, 320, 480, 640, 768)

    /**
     * Rounds a displayed width up to a shared decode size. Grid columns differ by a pixel of
     * rounding; exact widths fetched, decoded and cached the same cover once per column width.
     */
    fun decodeSize(pixels: Int): Int =
        decodeSizes.firstOrNull { it >= pixels } ?: decodeSizes.last()

    fun hasArtwork(media: MediaCard): Boolean =
        media.value.optString("thumbnail").isNotBlank() ||
            (media.value.optJSONArray("thumbnails")?.length() ?: 0) > 0

    /** Non-suspending memory lookup for the first frame; never decodes or fetches. */
    fun cached(media: MediaCard, pixels: Int): Bitmap? =
        memory.get("${media.key}:${media.artworkSource}:${decodeSize(pixels)}")

    suspend fun load(media: MediaCard, pixels: Int): Bitmap? {
        if (!hasArtwork(media)) return null
        val size = decodeSize(pixels)
        val key = "${media.key}:${media.artworkSource}:$size"
        memory.get(key)?.let {
            return it
        }
        return slots.withPermit {
            for (attempt in 0..2) {
                currentCoroutineContext().ensureActive()
                memory.get(key)?.let {
                    return@withPermit it
                }
                val decoded =
                    withContext(Dispatchers.IO) {
                        val data =
                            NativeBridge.thumbnail(media.value.toString(), size)
                                ?: return@withContext null
                        currentCoroutineContext().ensureActive()
                        if (
                            data.size < 3 ||
                                data[0] !in 1..768 ||
                                data[1] !in 1..768 ||
                                data.size != data[0] * data[1] + 2
                        )
                            return@withContext null
                        Bitmap.createBitmap(
                                data,
                                2,
                                data[0],
                                data[0],
                                data[1],
                                Bitmap.Config.ARGB_8888,
                            )
                            .also { memory.put(key, it) }
                    }
                if (decoded != null) return@withPermit decoded
                // A transient image failure must not leave a permanent placeholder.
                // Bound retries and release the coroutine promptly when a card leaves.
                if (attempt < 2) delay(200L shl attempt)
            }
            null
        }
    }
}

internal class YtuiViewModel : ViewModel() {
    val tab = MutableStateFlow(1)
    val feeds = Array(4) { MutableStateFlow(FeedState()) }
    private val history = Array(4) { ArrayDeque<FeedState>() }
    private val feedJobs = arrayOfNulls<Job>(4)

    private data class FeedKey(val tab: Int, val request: String, val network: Long)

    private val feedPages =
        FeedPageCache<FeedKey, JSONObject>(
            viewModelScope,
            android.os.SystemClock::elapsedRealtime,
            { it.toString().length * 2 },
        )
    private var categoryJob: Job? = null
    private var categoryFetch: Job? = null
    private var playbackJob: Job? = null
    private var latestPlaybackIntent = -1L
    private var lastVideoPreparation: PreparingVideo? = null
    private var seekTarget: Pair<String, Float>? = null
    private var seekDeadline = 0L
    private var accountEpoch = 0L
    private val settingsLock = Mutex()
    private val snapshotLock = Mutex()
    private var lastPlayer = ""
    private var lastDownloads = ""
    val mobile = MutableStateFlow(JSONObject())
    val config = MutableStateFlow(JSONObject())
    val palette = MutableStateFlow(JSONObject())
    val themes = MutableStateFlow<List<String>>(emptyList())
    val account = MutableStateFlow(JSONObject())
    val player = MutableStateFlow(PlayerState())
    val downloads = MutableStateFlow<List<JSONObject>>(emptyList())
    val offlinePlaylists = MutableStateFlow<List<JSONObject>>(emptyList())
    val queue = MutableStateFlow<List<MediaCard>>(emptyList())
    val upNext = MutableStateFlow(UpNextState())
    private var upNextJob: Job? = null
    private var shownRelated: Pair<String, List<MediaCard>>? = null
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
                    val preparing = player.value.preparing
                    val accepted =
                        preparing != null &&
                            it.optBoolean("video") &&
                            it.optJSONObject("media")?.optString("url") == preparing.media.url &&
                            it.optLong("startup_token") == preparing.token
                    // Deduplicate native snapshots without dropping a pending intent;
                    // acceptance and video mode still change in one state emission.
                    val raw = it.toString()
                    if (raw != lastPlayer || accepted) {
                        lastPlayer = raw
                        player.value = PlayerState(it, preparing.takeUnless { accepted })
                    }
                }
                value.optJSONArray("downloads")?.let {
                    val raw = it.toString()
                    if (raw != lastDownloads) {
                        lastDownloads = raw
                        downloads.value = it.objects()
                    }
                }
                value.optJSONArray("events")?.objects()?.forEach { event ->
                    val text = event.optString("text")
                    when (event.optString("kind")) {
                        "storage_updated" -> {
                            mobile.value = withContext(Dispatchers.IO) { MobileSettings.snapshot() }
                            OfflineStore.dirty = true
                        }
                        "playback_start_failed" -> {
                            val preparing =
                                lastVideoPreparation?.takeIf {
                                    it.token == latestPlaybackIntent &&
                                        it.token.toString() == event.optString("request_token")
                                }
                            if (preparing != null) {
                                player.update { current ->
                                    if (current.preparing?.token == preparing.token)
                                        current.copy(preparing = null)
                                    else current
                                }
                                if (playerOpen.value) playerOpen.value = preparing.restoreOpen
                                status.value = text
                                actionError.value = text
                                lastVideoPreparation = null
                            } else if (
                                latestPlaybackIntent >= 0 &&
                                    latestPlaybackIntent.toString() ==
                                        event.optString("request_token")
                            ) {
                                // Audio and previews have no preparing-video panel,
                                // but their current start failures must remain visible.
                                status.value = text
                                actionError.value = text
                            }
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
                        "history_updated" -> {
                            loadHistory()
                            loadUpNext()
                        }
                        "up_next_changed" -> loadUpNext()
                        "history_cleared" -> loadHistory()
                        "lyrics_file" ->
                            if (event.optString("request_token") == lyricToken)
                                importedLyrics.value = text
                    }
                }
            }
        }

    fun chooseTab(index: Int) {
        categoryJob?.cancel()
        tab.value = index
        if (index > 0 && feeds[index].value.generation == 0L && !MediaBridge.offlineUi)
            navigate(index, if (index == 3) "Cliamp" else "Home", remember = false)
        else if (index in 1..2 && !feeds[index].value.loading) preloadCategories(index)
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
        categoryJob?.cancel()
        val epoch = accountEpoch
        val network = NetworkSettings.revision.get()
        val categoryNavigation =
            (request as? JSONObject)?.let { it.has("Category") || it.has("MusicSearch") } == true
        val cacheable =
            index in 1..2 &&
                !append &&
                request != "Favorites" &&
                (request as? JSONObject)?.has("SavedMedia") != true
        val cacheKey = FeedKey(index, request.toString(), network)
        // Selecting the current source again is an explicit refresh.
        if (
            cacheable &&
                previous.generation > 0 &&
                !previous.loading &&
                previous.request.toString() == request.toString()
        )
            feedPages.invalidate(cacheKey)
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
            else
                FeedState(
                    request = request,
                    categories = if (categoryNavigation) previous.categories else emptyList(),
                    loading = true,
                    generation = generation,
                )
        feeds[index].value = loading
        feedJobs[index] =
            viewModelScope.launch {
                try {
                    val fetch: suspend () -> JSONObject = {
                        RustRpc.objectCall(
                            "browse",
                            "tab" to index,
                            "request" to request,
                            "offset" to if (append) previous.cards.size else 0,
                        )
                    }
                    val page = if (cacheable) feedPages.load(cacheKey, fetch) else fetch()
                    if (feeds[index].value.generation != generation) return@launch
                    if (epoch != accountEpoch) return@launch
                    if (network != NetworkSettings.revision.get()) {
                        navigate(index, request, append, remember = false)
                        return@launch
                    }
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
                                else
                                    (page.optJSONArray("categories")?.objects() ?: emptyList())
                                        .ifEmpty {
                                            if (categoryNavigation) previous.categories
                                            else emptyList()
                                        },
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
                    if (!append && index == tab.value) preloadCategories(index)
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

    /** One speculative request at a time; opening it joins the same fetch. */
    private fun preloadCategories(index: Int) {
        categoryJob?.cancel()
        if (index !in 1..2 || !categoryPreloadAllowed()) return
        val feed = feeds[index].value
        val requests = feed.categories.mapNotNull { it.opt("request") }
        val selected = requests.indexOfFirst { it.toString() == feed.request.toString() }
        val nearby =
            requests.indices
                .sortedBy { kotlin.math.abs(it - selected.coerceAtLeast(0)) }
                .map { requests[it] }
                .filter { it.toString() != feed.request.toString() }
                .take(3)
        val targets =
            buildList<Pair<Int, Any>> {
                    nearby.forEach { add(index to it) }
                    if (feed.request != "Home") add(index to "Home")
                    add((if (index == 1) 2 else 1) to "Home")
                    if (account.value.optBoolean("connected")) {
                        add(index to "Playlists")
                        add(index to "Liked")
                    }
                }
                .distinctBy { "${it.first}:${it.second}" }
                .take(6)
        val epoch = accountEpoch
        val network = NetworkSettings.revision.get()
        categoryJob =
            viewModelScope.launch {
                delay(300) // Let visible content and touch handling settle first.
                for ((target, request) in targets) {
                    // A cancelled batch may still have one fetch that a tap can reuse.
                    // Wait for it before starting more speculative network work.
                    categoryFetch?.join()
                    if (
                        !categoryPreloadAllowed() ||
                            epoch != accountEpoch ||
                            network != NetworkSettings.revision.get()
                    )
                        return@launch
                    try {
                        feedPages.load(FeedKey(target, request.toString(), network)) {
                            val work = currentCoroutineContext()[Job]
                            categoryFetch = work
                            try {
                                RustRpc.objectCall(
                                    "browse",
                                    "tab" to target,
                                    "request" to request,
                                    "offset" to 0,
                                )
                            } finally {
                                if (categoryFetch == work) categoryFetch = null
                            }
                        }
                    } catch (cancelled: CancellationException) {
                        throw cancelled
                    } catch (_: Exception) {
                        // Stop on any failure/cooldown; background work never retries or opens
                        // errors.
                        return@launch
                    }
                }
            }
    }

    private fun categoryPreloadAllowed(): Boolean {
        if (
            MediaBridge.offlineUi ||
                !mobile.value.optBoolean("preload_next", true) ||
                playbackJob?.isActive == true ||
                player.value.playing ||
                playerOpen.value
        )
            return false
        val connectivity =
            MediaBridge.context.getSystemService(android.content.Context.CONNECTIVITY_SERVICE)
                as? android.net.ConnectivityManager ?: return false
        return mobile.value.optBoolean("preload_mobile", false) ||
            !connectivity.isActiveNetworkMetered
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
        categoryJob?.cancel()
        feeds[index].value =
            history[index]
                .removeLast()
                .copy(generation = feeds[index].value.generation + 1, loading = false)
        if (index in 1..2) preloadCategories(index)
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

    private fun videoId(media: MediaCard): String? =
        media.value.optString("id").takeIf { it.matches(Regex("[A-Za-z0-9_-]{11}")) }
            ?: Uri.parse(media.url)
                .let { uri ->
                    uri.getQueryParameter("v")
                        ?: if (uri.host == "youtu.be") uri.lastPathSegment else null
                }
                ?.takeIf { it.matches(Regex("[A-Za-z0-9_-]{11}")) }

    private fun context(media: MediaCard, forceRelated: Boolean = false): JSONObject? {
        if (media.live || media.collection) return null
        val feed = feeds[tab.value].value
        val request = feed.request as? JSONObject
        val finite =
            !forceRelated &&
                request != null &&
                listOf("Collection", "Details", "CliampTracks").any(request::has)
        val id = videoId(media)
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

    /** Prepare at most one settled row, off the touch path and without history/queue effects. */
    suspend fun prepareVisible(cards: List<MediaCard>) {
        if (
            playbackJob?.isActive == true ||
                playerOpen.value ||
                player.value.value.optBoolean("playing")
        )
            return
        val candidates =
            cards.filter { !it.collection && !it.live && PlaybackCache.allowed(it.value) }.take(3)
        val quality = config.value.optJSONObject("download")?.optString("quality", "1080") ?: "1080"
        for ((index, media) in candidates.withIndex()) {
            if (
                playbackJob?.isActive == true ||
                    playerOpen.value ||
                    player.value.value.optBoolean("playing")
            )
                return
            try {
                val account = NativeBridge.accountIntent()
                val network = NetworkSettings.revision.get()
                val resolved =
                    RustRpc.objectCall(
                        "resolve",
                        "media" to media.value,
                        "audio" to media.audio,
                        "quality" to quality,
                        "speculative" to true,
                    )
                if (playbackJob?.isActive == true || playerOpen.value) return
                if (index == 0)
                    PlaybackCache.prefetchResolved(
                        media.value,
                        media.audio,
                        quality,
                        resolved,
                        account,
                        network,
                    )
            } catch (cancel: CancellationException) {
                throw cancel
            } catch (_: Exception) {
                return
            } // no background retry storm or error overlay
        }
    }

    /** UI preparation only: no source, queue/history acceptance or timing completion. */
    internal fun prepareVideoSurface(
        media: MediaCard,
        timing: Long,
        restoreOpen: Boolean = playerOpen.value && (player.value.preparing?.restoreOpen ?: true),
    ) {
        val preparation = PreparingVideo(timing, media, restoreOpen)
        latestPlaybackIntent = timing
        lastVideoPreparation = preparation
        player.update { it.copy(preparing = preparation) }
        playerOpen.value = true
    }

    fun play(
        media: MediaCard,
        preview: Boolean = false,
        download: Boolean = false,
        resume: Boolean = false,
        queued: Boolean = false,
        related: Boolean = false,
        continueContext: String? = null,
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
        val timing =
            if (!download && !preview)
                StartupTimings.begin(android.os.SystemClock.elapsedRealtime(), !audio)
            else 0L
        if (!download) {
            latestPlaybackIntent = timing
            if (audio || preview) lastVideoPreparation = null
        }
        if (timing != 0L) StartupTimings.playerCache(timing, PlaybackService.warmReady())
        if (timing != 0L)
            StartupTimings.byteCache(timing, PlaybackCache.readyFor(media.value, audio, quality))
        val context =
            if (!preview && !download && !queued && continueContext == null)
                context(media, related)
            else null
        // Desktop parity (`resume_on_play`): ordinary video plays continue from the
        // saved position; music, live media, previews and downloads start at 0.
        val autoResume =
            !resume &&
                !preview &&
                !download &&
                !audio &&
                !media.live &&
                mobile.value.optBoolean("resume_on_play", true)
        val epoch = accountEpoch
        val network = NetworkSettings.revision.get()
        val prepareSurface = !download && !preview && !audio
        val restoreOpen = playerOpen.value && (player.value.preparing?.restoreOpen ?: true)
        if (!download) playbackJob?.cancel()
        if (!download) PlaybackCache.pauseBytes()
        if (prepareSurface) prepareVideoSurface(media, timing, restoreOpen)
        val job = task {
            var handedOff = false
            try {
                actionError.value = null
                status.value = if (download) "Preparing download…" else "Resolving playback…"
                val offlineAt = android.os.SystemClock.elapsedRealtime()
                val local =
                    if (!download && !preview && mobile.value.optBoolean("prefer_offline", true))
                        withContext(Dispatchers.IO) {
                            OfflineStore.find(media.value, audio, quality)
                        }
                    else null
                StartupTimings.stage(
                    timing,
                    "offline",
                    android.os.SystemClock.elapsedRealtime() - offlineAt,
                )
                val stream =
                    local?.optJSONObject("stream")
                        ?: RustRpc.objectCall(
                            "resolve",
                            "startup_token" to timing,
                            "media" to media.value,
                            "audio" to audio,
                            "quality" to quality,
                        )
                if (epoch != accountEpoch || network != NetworkSettings.revision.get()) {
                    StartupTimings.ended(
                        timing,
                        "cancelled",
                        android.os.SystemClock.elapsedRealtime(),
                    )
                    return@task
                }
                StartupTimings.resolved(
                    timing,
                    android.os.SystemClock.elapsedRealtime(),
                    local != null || stream.optBoolean("resolution_cache_hit"),
                )
                if (local == null) MediaBridge.recordResolutionTimings(timing, stream)
                StartupTimings.route(
                    timing,
                    if (local != null) "offline"
                    else stream.optString("resolution_route", "direct-media"),
                )
                val item = media.value.copyJson()
                PlaybackService.mergeMetadata(item, stream)
                val request =
                    (local ?: JSONObject())
                        .apply { put("media", item) }
                        .apply {
                            put("stream", stream)
                            put("startup_token", timing)
                            put("audio", audio)
                            put("quality", quality)
                            put("preview", preview)
                            put("resume", resume || autoResume)
                            put("auto_resume", autoResume)
                            put("queued", queued)
                            put("context_id", continueContext ?: context?.optString("context_id"))
                            put("context", context)
                            if (continueContext != null) {
                                put("context_entry", true)
                                put("context_skip", true)
                            }
                        }
                /* request retains the cache-only source marker when saved offline. */
                effect(if (download) "download" else "play", "request" to request)
                if (prepareSurface) {
                    val accepted =
                        withTimeoutOrNull(5000) {
                            while (
                                PlaybackService.current?.startupToken != timing &&
                                    player.value.preparing?.token == timing
                            ) delay(16)
                            PlaybackService.current?.startupToken == timing ||
                                player.value.value.optLong("startup_token") == timing
                        } == true
                    if (!accepted)
                        throw RpcError(
                            "playback_start",
                            actionError.value ?: "Player could not start. Try again.",
                        )
                }
                handedOff = true
                if (
                    !download &&
                        !prepareSurface &&
                        (!audio ||
                            config.value.optString("music_player_style").endsWith("-fullscreen"))
                )
                    playerOpen.value = true
                status.value = if (download) "Download queued" else "Starting playback"
            } finally {
                if (!handedOff && prepareSurface) {
                    StartupTimings.ended(
                        timing,
                        if (isActive) "failed" else "cancelled",
                        android.os.SystemClock.elapsedRealtime(),
                    )
                    val own = player.value.preparing?.takeIf { it.token == timing }
                    if (own != null) {
                        player.update {
                            if (it.preparing?.token == timing) it.copy(preparing = null) else it
                        }
                        if (playerOpen.value) playerOpen.value = own.restoreOpen
                    }
                }
            }
        }
        if (!download) playbackJob = job
    }

    fun effect(op: String, vararg fields: Pair<String, Any?>) {
        MediaBridge.execute(json("op" to op, *fields).toString())
    }

    fun command(op: String, vararg fields: Pair<String, Any?>) {
        // Optimistic local state below must be reconciled by the next real snapshot,
        // even if the bridge reports the same state as before.
        lastPlayer = ""
        if (op == "seek") {
            val seconds =
                (fields.firstOrNull { it.first == "seconds" }?.second as? Number)?.toFloat()
            if (seconds == null || !seconds.isFinite() || player.value.duration <= 0) return
            val target = seconds.coerceIn(0f, player.value.duration)
            seekTarget = (player.value.media?.optString("url") ?: "") to target
            seekDeadline = android.os.SystemClock.elapsedRealtime() + 2000
            player.update { current ->
                current.copy(value = current.value.copyJson().put("position", target))
            }
        }
        if (op == "pause")
            player.update { current ->
                current.copy(
                    value =
                        current.value
                            .copyJson()
                            .put("playing", !current.playing)
                            .put("paused", current.playing)
                )
            }
        if (op == "volume" || op == "speed") {
            val value = fields.firstOrNull { it.first == "value" }?.second as? Number
            if (value != null && value.toDouble().isFinite()) {
                player.update { current ->
                    current.copy(value = current.value.copyJson().put(op, value))
                }
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

    /** Mirrors Next: the manual queue first, then the playlist tail or related videos. */
    fun loadUpNext() {
        upNextJob?.cancel()
        upNextJob =
            viewModelScope.launch {
                val media = player.value.media?.let(::MediaCard)
                val view =
                    runCatching { RustRpc.objectCall("up_next") }.getOrNull() ?: return@launch
                queue.value = view.optJSONArray("queue")?.objects()?.map(::MediaCard) ?: emptyList()
                val context = view.optJSONObject("context")
                val items =
                    context?.optJSONArray("items")?.objects()?.map(::MediaCard) ?: emptyList()
                val id = context?.optString("id").orEmpty()
                upNext.value =
                    if (items.isNotEmpty())
                        UpNextState(
                            items,
                            contextId = id,
                            related = context?.optBoolean("related") == true,
                            more = (context?.optInt("count") ?: 0) > items.size,
                        )
                    else
                        UpNextState(
                            loading = context != null && !context.optBoolean("prepared"),
                        )
                if (items.isNotEmpty() || media == null) return@launch
                // Related autoplay is off (or unavailable here): still show recommendations.
                if (
                    mobile.value.optBoolean("autoplay_related", true) ||
                        media.live ||
                        MediaBridge.offlineUi ||
                        player.value.value.optBoolean("offline")
                )
                    return@launch
                val videoId = videoId(media) ?: return@launch
                val cached = shownRelated?.takeIf { it.first == media.url }?.second
                val related =
                    cached
                        ?: runCatching {
                                RustRpc.objectCall(
                                        "browse",
                                        "tab" to if (media.audio) 2 else 1,
                                        "request" to json("Related" to videoId),
                                    )
                                    .optJSONArray("items")
                                    ?.objects()
                                    ?.map(::MediaCard)
                                    ?.filter { !it.collection && !it.live && it.url != media.url }
                                    ?.take(25)
                            }
                            .getOrNull()
                        ?: return@launch
                shownRelated = media.url to related
                if (player.value.media?.optString("url") == media.url)
                    upNext.value = UpNextState(related, related = true, autoplay = false)
            }
    }

    /** Plays an Up next row the way Next would have reached it. */
    fun playUpNext(media: MediaCard, fromQueue: Boolean) {
        val state = upNext.value
        when {
            fromQueue -> play(media, queued = true)
            state.related -> play(media, related = true)
            state.contextId.isNotBlank() ->
                play(media, queued = true, continueContext = state.contextId)
            else -> play(media)
        }
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
            val browseChanged =
                draft.optInt("account_index") != config.value.optInt("account_index") ||
                    draft.optString("proxy") != config.value.optString("proxy")
            RustRpc.objectCall("save_settings", "config" to draft)
            val appliedPalette = RustRpc.objectCall("theme", "dark" to dark)
            palette.value = appliedPalette
            config.value = draft
            effect("settings", "config" to draft)
            if (browseChanged) {
                invalidateAccount()
                if (!MediaBridge.offlineUi && tab.value in 1..2)
                    navigate(request = "Home", remember = false)
            }
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
        categoryJob?.cancel()
        feedPages.clear()
        accountEpoch++
        latestPlaybackIntent = -1L
        lastVideoPreparation = null
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
        categoryJob?.cancel()
        feedPages.clear()
        playbackJob?.cancel()
        feedJobs.forEach { it?.cancel() }
        super.onCleared()
    }
}
