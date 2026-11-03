package dev.ytui.android

import android.view.View
import android.widget.TextView
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.platform.ViewCompositionStrategy
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import androidx.media3.ui.PlayerView
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Every playback option from the main player, inside fullscreen. The panel is a Compose layer in
 * the fullscreen window itself (no dialog window), so system bars stay hidden and the shared
 * decoder keeps rendering behind it. Options reuse [PlayerPane] and the same Rust commands.
 */
@androidx.annotation.OptIn(markerClass = [androidx.media3.common.util.UnstableApi::class])
internal object FullscreenOptions {
    /** Test hook: whether the fullscreen options panel is currently open. */
    @JvmStatic
    val open = MutableStateFlow(false)

    @JvmStatic
    fun attach(activity: ComponentActivity, view: PlayerView): View {
        open.value = false
        view.findViewById<View>(R.id.ytui_heading)?.visibility = View.VISIBLE
        view.findViewById<View>(R.id.ytui_settings)?.apply {
            visibility = View.VISIBLE
            setOnClickListener {
                view.hideController()
                open.value = true
            }
        }
        // Share the main screen's state (settings, queue, palette) when it is alive; a
        // standalone model still drives the same playback service through the bridge.
        val owner: ComponentActivity =
            RustActivity.current()?.takeUnless { it.isDestroyed || it.isFinishing } ?: activity
        val vm = ViewModelProvider(owner)[YtuiViewModel::class.java]
        return ComposeView(activity).apply {
            setViewCompositionStrategy(ViewCompositionStrategy.DisposeOnViewTreeLifecycleDestroyed)
            setContent { FullscreenOptionsLayer(vm, view, activity::finish) }
        }
    }
}

/** Blur (radius in px) the video layer under the options panel; no-op before Android 12. */
private fun blurVideo(content: View?, radius: Float) {
    if (content == null || android.os.Build.VERSION.SDK_INT < 31) return
    content.setRenderEffect(
        if (radius < 0.5f) null
        else
            android.graphics.RenderEffect.createBlurEffect(
                radius,
                radius,
                android.graphics.Shader.TileMode.CLAMP,
            )
    )
}

@androidx.annotation.OptIn(markerClass = [androidx.media3.common.util.UnstableApi::class])
@Composable
private fun FullscreenOptionsLayer(vm: YtuiViewModel, view: PlayerView, exit: () -> Unit) {
    val visible by FullscreenOptions.open.collectAsState()
    val player by vm.player.collectAsState()
    val config by vm.config.collectAsState()
    val mobile by vm.mobile.collectAsState()
    val queue by vm.queue.collectAsState()
    val upNext by vm.upNext.collectAsState()
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    // The main activity is stopped beneath fullscreen, so this window keeps the
    // player snapshot current (same cadence and signal as the main screen).
    LaunchedEffect(lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            coroutineScope {
                launch { ComposeHost.signals.collect { vm.snapshot() } }
                launch {
                    while (isActive) {
                        vm.snapshot()
                        delay(1000)
                    }
                }
            }
        }
    }
    val title = player.title
    val creator =
        player.value.optString("station").ifBlank { player.media?.optString("creator") ?: "" }
    SideEffect {
        view.findViewById<TextView>(R.id.ytui_title)?.text = title
        view.findViewById<TextView>(R.id.ytui_subtitle)?.apply {
            text = creator
            visibility = if (creator.isBlank()) View.GONE else View.VISIBLE
        }
    }
    val scheme = ComposeHost.colorScheme ?: darkColorScheme()
    val reduced =
        config.optBoolean("reduced_motion") ||
            !android.animation.ValueAnimator.areAnimatorsEnabled()
    var page by remember { mutableStateOf("") }
    fun close() {
        FullscreenOptions.open.value = false
        page = ""
    }
    BackHandler(visible) { if (page.isNotEmpty()) page = "" else close() }
    // Frosted glass: blur the video picture itself behind the translucent panel.
    // RenderEffect blurs the TextureView's layer on the GPU (Android 12+); older
    // versions keep the darker scrim and a more opaque panel.
    val blurSupported = android.os.Build.VERSION.SDK_INT >= 31
    val blurTarget = with(LocalDensity.current) { if (visible) 28.dp.toPx() else 0f }
    val blur by
        animateFloatAsState(
            blurTarget,
            if (reduced) androidx.compose.animation.core.snap() else tween(220),
            label = "video-blur",
        )
    val content = view.findViewById<View>(androidx.media3.ui.R.id.exo_content_frame)
    SideEffect { blurVideo(content, blur) }
    DisposableEffect(content) { onDispose { blurVideo(content, 0f) } }
    MaterialTheme(colorScheme = scheme, typography = MediaTokens.typography) {
        CompositionLocalProvider(
            LocalContentColor provides scheme.onSurface,
            LocalReducedMotion provides reduced,
        ) {
            BoxWithConstraints(Modifier.fillMaxSize()) {
                val wide = maxWidth > maxHeight
                val sheetWidth = if (wide) minOf(420.dp, maxWidth * 0.5f) else maxWidth
                AnimatedVisibility(
                    visible,
                    enter = if (reduced) EnterTransition.None else fadeIn(tween(160)),
                    exit = if (reduced) ExitTransition.None else fadeOut(tween(120)),
                ) {
                    // Tapping the dimmed video closes the panel; it never reaches the player.
                    Box(
                        Modifier.fillMaxSize()
                            .background(Color.Black.copy(alpha = if (blurSupported) 0.28f else 0.5f))
                            .testTag("fullscreen-options-scrim")
                            .pointerInput(Unit) { detectTapGestures { close() } }
                    )
                }
                AnimatedVisibility(
                    visible,
                    Modifier.align(Alignment.CenterEnd),
                    enter =
                        if (reduced) EnterTransition.None
                        else slideInHorizontally(tween(220)) { it / 3 } + fadeIn(tween(160)),
                    exit =
                        if (reduced) ExitTransition.None
                        else slideOutHorizontally(tween(160)) { it / 3 } + fadeOut(tween(120)),
                ) {
                    Surface(
                        Modifier.width(sheetWidth)
                            .fillMaxHeight()
                            .testTag("fullscreen-options")
                            .semantics { paneTitle = "Playback options" },
                        shape = RoundedCornerShape(topStart = 28.dp, bottomStart = 28.dp),
                        color = scheme.surfaceContainerLow.copy(alpha = if (blurSupported) 0.72f else 0.92f),
                        border = BorderStroke(1.dp, MediaTokens.videoGlassEdge.copy(alpha = 0.18f)),
                    ) {
                        Column(
                            Modifier.fillMaxSize()
                                .background(
                                    Brush.verticalGradient(
                                        listOf(MediaTokens.videoGlassSheen, Color.Transparent),
                                        endY = 360f,
                                    )
                                )
                                .windowInsetsPadding(
                                    WindowInsets.displayCutout.only(
                                        WindowInsetsSides.End + WindowInsetsSides.Vertical
                                    )
                                )
                                .padding(start = 8.dp, end = 8.dp, top = 8.dp)
                        ) {
                            Row(
                                Modifier.fillMaxWidth().heightIn(min = 56.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                if (page.isNotEmpty())
                                    MediaIconButton(
                                        Glyph.Back,
                                        "Back to playback options",
                                        "fullscreen-options-back",
                                    ) {
                                        page = ""
                                    }
                                Text(
                                    when (page) {
                                        "sleep" -> "Sleep timer"
                                        "queue" -> "Up next"
                                        else -> "Playback options"
                                    },
                                    Modifier.weight(1f).padding(horizontal = 8.dp),
                                    style = MaterialTheme.typography.titleLarge,
                                    fontWeight = FontWeight.Medium,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                                MediaIconButton(
                                    Glyph.Close,
                                    "Close playback options",
                                    "fullscreen-options-close",
                                ) {
                                    close()
                                }
                            }
                            HorizontalDivider(color = scheme.outlineVariant)
                            when (page) {
                                "queue" ->
                                    UpNext(
                                        queue,
                                        upNext,
                                        player.media?.optString("url"),
                                        player.value.optBoolean("shuffle"),
                                        config.optBoolean("thumbnails", true),
                                        mobile.optBoolean("show_channel", true),
                                        Modifier.weight(1f),
                                        onManage = null,
                                        showHeader = false,
                                        play = { media, fromQueue ->
                                            vm.playUpNext(media, fromQueue)
                                            // Music has no picture: return to the regular player.
                                            if (media.audio) {
                                                close()
                                                exit()
                                            }
                                        },
                                    )
                                "sleep" ->
                                    Column(Modifier.verticalScroll(rememberScrollState())) {
                                        SleepOptions(vm) { page = "" }
                                    }
                                else ->
                                    Column(
                                        Modifier.weight(1f)
                                            .verticalScroll(rememberScrollState())
                                            .padding(bottom = 16.dp)
                                    ) {
                                        PlayerPane(
                                            vm,
                                            expanded = true,
                                            showSurface = false,
                                            onQueue = {
                                                vm.loadQueue()
                                                page = "queue"
                                            },
                                            onSleep = { page = "sleep" },
                                            onDone = {
                                                close()
                                                exit()
                                            },
                                        )
                                    }
                            }
                        }
                    }
                }
            }
        }
    }
}
