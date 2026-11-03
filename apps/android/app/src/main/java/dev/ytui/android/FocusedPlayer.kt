package dev.ytui.android

import android.content.Intent
import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.Player
import kotlin.math.abs

@androidx.annotation.OptIn(markerClass = [androidx.media3.common.util.UnstableApi::class])
@Composable
internal fun VideoSurface(
    modifier: Modifier = Modifier,
    onCollapse: (() -> Unit)? = null,
    playbackPlayer: Player? = PlaybackService.current?.controlsPlayer,
    covered: Boolean = false,
    timingToken: Long = PlaybackService.current?.startupToken ?: 0,
    onSettings: (() -> Unit)? = null,
) {
    val collapse by rememberUpdatedState(onCollapse)
    val settings by rememberUpdatedState(onSettings)
    val accent = MaterialTheme.colorScheme.primary.toArgb()
    val reduced by rememberUpdatedState(LocalReducedMotion.current)
    AndroidView(
        factory = { context ->
            val inflateAt = android.os.SystemClock.elapsedRealtime()
            VideoViewPreparation.take(context)
                .also {
                    StartupTimings.stage(
                        timingToken,
                        "view_inflate",
                        android.os.SystemClock.elapsedRealtime() - inflateAt,
                    )
                }
                .apply {
                    // Media3's workaround keeps SurfaceView geometry synchronized
                    // inside Compose's AndroidView on Android 14+.
                    setEnableComposeSurfaceSyncWorkaround(true)
                    useController = true
                    setControllerAnimationEnabled(
                        !reduced && android.animation.ValueAnimator.areAnimatorsEnabled()
                    )
                    setShowSubtitleButton(true)
                    // Queue modes stay in the app's transport; the video chrome is for watching.
                    setShowShuffleButton(false)
                    setFullscreenButtonClickListener {
                        context.startActivity(
                            Intent(context, FullscreenActivity::class.java)
                                .putExtra("reduced_motion", reduced)
                        )
                    }
                    tag = VisibleFrameTracker(this)
                    VideoGestures.attach(this)
                }
        },
        modifier = modifier.background(Color.Black).testTag("native-video-surface"),
        update = { view ->
            view.setControllerAnimationEnabled(
                !reduced && android.animation.ValueAnimator.areAnimatorsEnabled()
            )
            PlayerChrome.rememberAccent(accent)
            PlayerChrome.apply(view)
            view.findViewById<android.view.View>(R.id.ytui_collapse)?.apply {
                tag = "panel-close" // Native View tag; Compose tag remains on music/modal chrome.
                visibility =
                    if (collapse == null) android.view.View.GONE else android.view.View.VISIBLE
                setOnClickListener { collapse?.invoke() }
            }
            // The app's own options (quality, speed, volume, tracks, sleep, segments)
            // replace Media3's reduced settings popup.
            view.findViewById<android.view.View>(R.id.ytui_settings)?.apply {
                visibility =
                    if (settings == null) android.view.View.GONE else android.view.View.VISIBLE
                setOnClickListener {
                    view.hideController()
                    settings?.invoke()
                }
            }
            val actual = playbackPlayer
            (view.tag as VisibleFrameTracker).setPresentable(!covered)
            (view.tag as VisibleFrameTracker).bind(actual)
            view.keepScreenOn = actual?.playWhenReady == true
        },
        onRelease = {
            it.keepScreenOn = false
            (it.tag as VisibleFrameTracker).bind(null)
        },
    )
}

/** A bounded screen: advanced controls open separately, never below a long cover. */
@Composable
internal fun FocusedPlayer(
    vm: YtuiViewModel,
    allowVideoSurface: Boolean,
    minimize: () -> Unit,
    onQueue: () -> Unit,
    onMore: () -> Unit,
    onLyrics: () -> Unit = {},
    onSleep: () -> Unit = {},
) {
    val player by vm.player.collectAsState()
    val config by vm.config.collectAsState()
    val mobile by vm.mobile.collectAsState()
    val queue by vm.queue.collectAsState()
    val upNext by vm.upNext.collectAsState()
    val context = LocalContext.current
    val mediaUrl = player.media?.optString("url")
    // Keep the inline Up next list current as playback advances.
    LaunchedEffect(mediaUrl) { vm.loadUpNext() }
    val threshold = with(LocalDensity.current) { 72.dp.toPx() }
    val motion = LocalMotion.current
    val panelDrag = LocalPanelDrag.current
    val restingDrag = remember { Animatable(0f) }
    // Music follows the finger through the panel; video only recognises the gesture so the
    // native surface is never moved by a Compose layer.
    val dismiss =
        Modifier.swipeAxis(
            enabled = mobile.gesture("gesture_swipe_dismiss"),
            horizontal = false,
            offset = if (!player.video && panelDrag != null) panelDrag.offset else restingDrag,
            threshold = threshold,
            follow = !player.video && panelDrag != null,
            allow = { it > 0 },
            onSwipe = { direction ->
                if (!player.video && panelDrag != null) panelDrag.dismiss(this, direction)
                else minimize()
            },
        )
    // Horizontal swipe on music artwork changes track, carousel style.
    val trackShift = remember { Animatable(0f) }
    val haptic = rememberGestureHaptic()
    val trackWidth =
        with(LocalDensity.current) { LocalConfiguration.current.screenWidthDp.dp.toPx() }
    val trackSwipe =
        Modifier.swipeAxis(
            enabled = !player.video && mobile.gesture("gesture_swipe_tracks"),
            horizontal = true,
            offset = trackShift,
            threshold = with(LocalDensity.current) { 80.dp.toPx() },
            onSwipe = { direction ->
                val out = direction * trackWidth * 0.6f
                trackShift.animateTo(out, motion.exit(200))
                vm.command(if (direction < 0) "next" else "previous")
                trackShift.snapTo(-out)
                trackShift.animateTo(0f, motion.spring())
            },
        )
    val artTaps =
        Modifier.pointerInput(mobile.gesture("gesture_double_tap_seek")) {
            if (mobile.gesture("gesture_double_tap_seek"))
                detectTapGestures(
                    onDoubleTap = {
                        haptic()
                        vm.command("pause")
                    }
                )
        }
    BoxWithConstraints(Modifier.fillMaxSize().testTag("focused-player")) {
        val landscape = maxWidth > maxHeight * 1.25f
        val portraitVideoHeight =
            minOf(maxWidth * (9f / 16f), (maxHeight - 60.dp).coerceAtLeast(1.dp))
        @Composable
        fun Controls(modifier: Modifier) {
            val preparing = player.preparing
            if (preparing != null) {
                Column(
                    modifier
                        .padding(horizontal = MediaTokens.textInset)
                        .heightIn(min = 48.dp)
                        .testTag("panel-header")
                        .then(dismiss)
                ) {
                    Text(
                        preparing.media.title,
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.Bold,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (preparing.media.creator.isNotBlank())
                        Text(
                            preparing.media.creator,
                            maxLines = 1,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                }
                return
            }
            Column(
                modifier
                    .padding(horizontal = MediaTokens.textInset)
                    .then(
                        if (player.video || LocalDensity.current.fontScale > 1.3f)
                            Modifier.verticalScroll(rememberScrollState())
                        else Modifier
                    ),
                verticalArrangement = Arrangement.spacedBy(if (landscape) 4.dp else 8.dp),
            ) {
                Column(
                    Modifier.fillMaxWidth()
                        .then(
                            if (player.video)
                                Modifier.heightIn(min = 48.dp).testTag("panel-header").then(dismiss)
                            else Modifier
                        ),
                    verticalArrangement = Arrangement.spacedBy(if (landscape) 4.dp else 8.dp),
                ) {
                    Text(
                        player.title,
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.Bold,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (
                        mobile.optBoolean("show_channel", true) ||
                            player.media?.optBoolean("live") == true
                    )
                        Text(
                            player.value.optString("station").ifBlank {
                                player.media?.optString("creator") ?: ""
                            },
                            style = MaterialTheme.typography.bodyMedium,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                }
                if (player.video) player.media?.let { MediaInformation(it, mobile) }
                player.value
                    .optString("error")
                    .takeUnless { it.isBlank() || it == "null" }
                    ?.let { Text(it, color = MaterialTheme.colorScheme.error, maxLines = 2) }
                // One row of existing actions under the title (YouTube watch-page
                // pattern). Every entry maps to an action the app already performs.
                // Narrow columns (landscape watch split) scroll instead of overlapping.
                val media = player.media
                val canSave =
                    media != null &&
                        !player.value.optBoolean("offline") &&
                        !media.optBoolean("live") &&
                        !player.preview
                val actionCount =
                    3 +
                        (if (!player.video) 1 else 0) +
                        (if (canSave) 1 else 0) +
                        (if (player.video) 1 else 0)
                BoxWithConstraints(Modifier.fillMaxWidth()) {
                    val minimum = 48.dp * actionCount
                    Row(
                        (if (maxWidth < minimum)
                                Modifier.horizontalScroll(rememberScrollState())
                                    .widthIn(min = minimum)
                            else Modifier.fillMaxWidth())
                            .testTag("player-actions"),
                        horizontalArrangement = Arrangement.SpaceBetween,
                    ) {
                        MediaIconButton(Glyph.Queue, "Playback queue", action = onQueue)
                        if (!player.video)
                            MediaIconButton(Glyph.Lyrics, "Lyrics", action = onLyrics)
                        MediaIconButton(Glyph.Sleep, "Sleep timer", action = onSleep)
                        if (canSave && media != null)
                            MediaIconButton(Glyph.Download, "Save offline", "player-download") {
                                vm.play(MediaCard(media), download = true)
                            }
                        if (player.video)
                            MediaIconButton(Glyph.Expand, "Fullscreen video", "fullscreen-button") {
                                context.startActivity(
                                    Intent(context, FullscreenActivity::class.java)
                                        .putExtra(
                                            "reduced_motion",
                                            config.optBoolean("reduced_motion"),
                                        )
                                )
                            }
                        MediaIconButton(
                            Glyph.More,
                            "Playback options",
                            "player-options",
                            action = onMore,
                        )
                    }
                }
                if (!player.video) PlayerSeek(vm, player)
                if (!player.video)
                    Row(
                        Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.SpaceEvenly,
                    ) {
                        MediaModeButton(
                            Glyph.Shuffle,
                            "Shuffle",
                            "player-shuffle",
                            player.value.optBoolean("shuffle"),
                        ) {
                            vm.command("shuffle")
                        }
                        MediaIconButton(Glyph.Previous, "Previous track") { vm.command("previous") }
                        FilledIconButton(
                            { vm.command("pause") },
                            Modifier.size(64.dp).testTag("player-pause").semantics {
                                contentDescription = if (player.playing) "Pause" else "Play"
                            },
                            colors =
                                IconButtonDefaults.filledIconButtonColors(
                                    containerColor = MaterialTheme.colorScheme.onBackground,
                                    contentColor = MaterialTheme.colorScheme.background,
                                ),
                        ) {
                            PlayPauseIcon(player.playing, Modifier.size(32.dp))
                        }
                        MediaIconButton(Glyph.Next, "Next track", "player-next") {
                            vm.command("next")
                        }
                        MediaModeButton(
                            Glyph.Repeat,
                            "Repeat one",
                            "player-repeat",
                            player.value.optBoolean("repeat_one"),
                        ) {
                            vm.command("repeat")
                        }
                    }
            }
        }
        @Composable
        fun Art(modifier: Modifier) {
            BoxWithConstraints(
                modifier.then(
                    if (player.video) Modifier else dismiss.then(trackSwipe).then(artTaps)
                ),
                // Music art shares spare height above and below instead of
                // leaving one gap between the cover and its title.
                contentAlignment = if (player.video) Alignment.TopCenter else Alignment.Center,
            ) {
                if (player.video && allowVideoSurface) {
                    Box(
                        Modifier.width(minOf(maxWidth, maxHeight * (16f / 9f)))
                            .aspectRatio(16f / 9f)
                    ) {
                        VideoSurface(
                            Modifier.fillMaxSize(),
                            onCollapse = minimize,
                            onSettings = onMore,
                            playbackPlayer = PlaybackService.current?.controlsPlayer,
                            covered = player.preparing != null,
                            timingToken =
                                player.preparing?.token ?: player.value.optLong("startup_token"),
                        )
                        player.preparing?.let { preparing ->
                            Box(
                                Modifier.fillMaxSize()
                                    .background(Color.Black)
                                    .testTag("preparing-video")
                                    .semantics { contentDescription = "Preparing video" },
                                contentAlignment = Alignment.Center,
                            ) {
                                Box(
                                    Modifier.matchParentSize().pointerInput(preparing.token) {
                                        awaitPointerEventScope {
                                            while (true) awaitPointerEvent().changes.forEach {
                                                it.consume()
                                            }
                                        }
                                    }
                                )
                                CircularProgressIndicator()
                                Box(Modifier.align(Alignment.TopStart).padding(4.dp)) {
                                    CompositionLocalProvider(
                                        LocalContentColor provides MediaTokens.videoInk
                                    ) {
                                        MediaIconButton(
                                            Glyph.Down,
                                            "Minimize player",
                                            "panel-close",
                                            minimize,
                                        )
                                    }
                                }
                            }
                        }
                    }
                } else if (player.video) {
                    // The root's distinct mode flow can lag the player snapshot
                    // by one frame. Never create a native view in the music animation.
                    Spacer(Modifier.fillMaxSize().background(Color.Black))
                } else {
                    val ratio =
                        if (config.optString("music_player_style", "classic") == "classic") 1f
                        else 1.35f
                    Visualizer(
                        player,
                        config,
                        Modifier.width(minOf(maxWidth, maxHeight * ratio))
                            .aspectRatio(ratio)
                            .graphicsLayer {
                                translationX = trackShift.value
                                val away = (abs(trackShift.value) / trackWidth).coerceIn(0f, 1f)
                                alpha = 1f - away * 0.7f
                                rotationZ = trackShift.value / trackWidth * 6f
                            },
                    )
                }
            }
        }
        if (player.video) {
            // Watch page: edge-to-edge 16:9 video (capped so details stay usable in
            // short windows), details directly beneath, then the real queue.
            if (landscape) {
                Row(Modifier.fillMaxSize(), horizontalArrangement = Arrangement.spacedBy(24.dp)) {
                    Art(Modifier.weight(2f).fillMaxHeight())
                    Controls(Modifier.weight(1f).fillMaxHeight())
                }
            } else {
                Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Art(Modifier.fillMaxWidth().height(portraitVideoHeight))
                    Controls(Modifier.fillMaxWidth())
                    UpNext(
                        queue,
                        upNext,
                        mediaUrl,
                        player.value.optBoolean("shuffle"),
                        config.optBoolean("thumbnails", true),
                        mobile.optBoolean("show_channel", true),
                        Modifier.weight(1f),
                        onQueue,
                        play = vm::playUpNext,
                    )
                }
            }
        } else if (landscape)
            Row(Modifier.fillMaxSize(), horizontalArrangement = Arrangement.spacedBy(24.dp)) {
                Art(Modifier.weight(1f).fillMaxHeight())
                Controls(Modifier.weight(1f).align(Alignment.CenterVertically))
            }
        else
            Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Art(Modifier.weight(1f).fillMaxWidth())
                Controls(Modifier.fillMaxWidth().padding(bottom = 8.dp))
            }
    }
}

/**
 * Inline Up next under the video and in the fullscreen options panel, in the order Next takes
 * them: the manual queue, then the rest of the playlist or related videos. Full queue management
 * stays in the queue sheet (`onManage == null` hides that entry).
 */
@Composable
internal fun UpNext(
    queue: List<MediaCard>,
    state: UpNextState,
    playingUrl: String?,
    shuffle: Boolean,
    thumbnails: Boolean,
    showCreator: Boolean,
    modifier: Modifier,
    onManage: (() -> Unit)?,
    showHeader: Boolean = true,
    play: (MediaCard, Boolean) -> Unit,
) {
    val upcoming = queue.filter { it.url.isBlank() || it.url != playingUrl }
    // The item Next/autoplay starts, unless shuffle makes the choice random.
    val next =
        if (shuffle) null
        else
            upcoming.firstOrNull()
                ?: state.context.firstOrNull { it.url != playingUrl }.takeIf { state.autoplay }
    Column(modifier.fillMaxWidth().testTag("up-next")) {
        if (showHeader) HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        if (showHeader)
            Row(
                Modifier.fillMaxWidth().padding(start = MediaTokens.textInset, end = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    "Up next",
                    Modifier.weight(1f),
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold,
                )
                if (queue.isNotEmpty() && onManage != null)
                    TextButton(
                        onManage,
                        Modifier.heightIn(min = 48.dp),
                        colors =
                            ButtonDefaults.textButtonColors(
                                contentColor = MaterialTheme.colorScheme.onSurface
                            ),
                    ) {
                        Text("Edit queue")
                    }
            }
        if (queue.isEmpty() && state.context.isEmpty())
            Text(
                if (state.loading) "Finding what plays next…"
                else "Nothing queued. Add videos with Add to queue in any ⋮ menu.",
                Modifier.padding(horizontal = MediaTokens.textInset, vertical = 8.dp),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        else
            androidx.compose.foundation.lazy.LazyColumn(
                Modifier.weight(1f).fillMaxWidth(),
                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                @Composable
                fun Entry(media: MediaCard, fromQueue: Boolean) {
                    val current = media.url.isNotBlank() && media.url == playingUrl
                    val label =
                        when {
                            current -> "Playing"
                            media === next -> "Plays next"
                            else -> ""
                        }
                    val creator = if (showCreator) media.creator else ""
                    MediaListRow(
                        media.title,
                        listOf(label, creator).filter { it.isNotBlank() }.joinToString(" · "),
                        Modifier.padding(horizontal = 8.dp),
                        artwork = { Artwork(media, it, thumbnails) },
                        square = media.audio,
                        highlighted = current,
                        onClick = { play(media, fromQueue) },
                    )
                }
                items(queue.size, key = { "${queue[it].key}#$it" }) { Entry(queue[it], true) }
                if (state.context.isNotEmpty()) {
                    item(key = "context-header") {
                        SectionHeader(
                            if (state.related) "Recommended" else "From this playlist",
                            Modifier.padding(horizontal = 8.dp).testTag("up-next-context"),
                        )
                    }
                    items(state.context.size, key = { "context:${state.context[it].key}#$it" }) {
                        Entry(state.context[it], false)
                    }
                    if (state.more)
                        item(key = "context-more") {
                            Text(
                                "More after these",
                                Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                }
            }
    }
}

@Composable
internal fun PlayerSeek(vm: YtuiViewModel, player: PlayerState) {
    val live = player.media?.optBoolean("live") == true
    var drag by remember(player.media?.optString("url")) { mutableStateOf<Float?>(null) }
    if (live || player.duration <= 0f) {
        Text(
            if (live) "LIVE" else if (player.value.optBoolean("buffering")) "Buffering…" else "",
            Modifier.heightIn(min = 36.dp),
            color = MaterialTheme.colorScheme.primary,
            style = MaterialTheme.typography.labelMedium,
        )
    } else {
        Column {
            MediaSeekSlider(
                (drag ?: player.position).coerceIn(0f, player.duration),
                { drag = it },
                valueRange = 0f..player.duration,
                onValueChangeFinished = {
                    drag?.let { vm.command("seek", "seconds" to it) }
                    drag = null
                },
                modifier = Modifier.fillMaxWidth().testTag("seek-slider"),
                marks = player.sponsorMarks(),
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Text(time(drag ?: player.position), style = MaterialTheme.typography.labelMedium)
                Text(time(player.duration), style = MaterialTheme.typography.labelMedium)
            }
        }
    }
}

/** SponsorBlock segments as (start, end) seconds for drawing on the seek timeline. */
internal fun PlayerState.sponsorMarks(): List<Pair<Float, Float>> =
    value.optJSONArray("segments")?.objects()?.mapNotNull { mark ->
        val start = mark.optDouble("start").toFloat()
        val end = mark.optDouble("end").toFloat()
        if (start.isFinite() && end.isFinite() && end > start) start to end else null
    } ?: emptyList()

/**
 * Keep Material's seek semantics/drag handling; draw a slim line and persistent round playhead.
 * [marks] are (start, end) values in [valueRange] highlighted on the track, e.g. sponsor segments.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun MediaSeekSlider(
    value: Float,
    onValueChange: (Float) -> Unit,
    valueRange: ClosedFloatingPointRange<Float>,
    onValueChangeFinished: () -> Unit,
    modifier: Modifier = Modifier,
    marks: List<Pair<Float, Float>> = emptyList(),
) {
    val accent = MaterialTheme.colorScheme.primary
    val track = MaterialTheme.colorScheme.outlineVariant
    val markColor = MaterialTheme.colorScheme.tertiary
    Slider(
        value,
        onValueChange,
        modifier.heightIn(min = 48.dp),
        valueRange = valueRange,
        onValueChangeFinished = onValueChangeFinished,
        thumb = { Box(Modifier.size(12.dp).clip(CircleShape).background(accent)) },
        track = { state ->
            Canvas(Modifier.fillMaxWidth().height(3.dp)) {
                val fraction =
                    mediaProgress(
                        state.value - state.valueRange.start,
                        state.valueRange.endInclusive - state.valueRange.start,
                    )
                drawLine(
                    track,
                    androidx.compose.ui.geometry.Offset(0f, size.height / 2),
                    androidx.compose.ui.geometry.Offset(size.width, size.height / 2),
                    size.height,
                )
                drawLine(
                    accent,
                    androidx.compose.ui.geometry.Offset(0f, size.height / 2),
                    androidx.compose.ui.geometry.Offset(size.width * fraction, size.height / 2),
                    size.height,
                )
                val span = state.valueRange.endInclusive - state.valueRange.start
                if (span > 0f) {
                    marks.forEach { (start, end) ->
                        val from = ((start - state.valueRange.start) / span).coerceIn(0f, 1f)
                        val to = ((end - state.valueRange.start) / span).coerceIn(from, 1f)
                        if (to > from) {
                            val y = size.height / 2
                            drawLine(
                                markColor,
                                androidx.compose.ui.geometry.Offset(size.width * from, y),
                                androidx.compose.ui.geometry.Offset(size.width * to, y),
                                size.height,
                            )
                        }
                    }
                }
            }
        },
    )
}
