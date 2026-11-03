package dev.ytui.android

import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.expandHorizontally
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.shrinkHorizontally
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.collectAsState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathMeasure
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import kotlin.math.max
import kotlinx.coroutines.delay

/** First-run setup is shown once per [VERSION]; bump it to re-run the flow for everyone. */
internal object OnboardingStore {
    const val VERSION = 1
    private const val PREFS = "onboarding"
    private const val KEY = "completed_version"

    fun shouldShow(context: Context): Boolean =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getInt(KEY, 0) < VERSION

    fun complete(context: Context) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putInt(KEY, VERSION).apply()
    }

    /** Lets "Replay setup" show the flow again. */
    fun reset(context: Context) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().remove(KEY).apply()
    }
}

/** Pure step navigation so it can be unit tested without Compose. */
internal object OnboardingSteps {
    const val COUNT = 5
    const val WELCOME = 0
    const val LOOK = 1
    const val PLAYBACK = 2
    const val ACCOUNT = 3
    const val READY = 4

    fun clamp(step: Int): Int = step.coerceIn(0, COUNT - 1)

    fun next(step: Int): Int = clamp(clamp(step) + 1)

    fun previous(step: Int): Int = clamp(clamp(step) - 1)

    fun isLast(step: Int): Boolean = clamp(step) == COUNT - 1

    /** Back goes to the previous page; on the first page it leaves setup (acts as skip). */
    fun backTarget(step: Int): Int? = if (clamp(step) == 0) null else previous(step)

    /** The last page has its own finish button, so the Skip label is hidden there. */
    fun showSkip(step: Int): Boolean = !isLast(step)

    fun isForward(from: Int, to: Int): Boolean = to > from

    fun stateDescription(step: Int): String = "Step ${clamp(step) + 1} of $COUNT"

    /**
     * Page after a horizontal drag of [totalDrag] pixels. Dragging towards the leading edge moves
     * forward (left in LTR, right in RTL). Drags below [threshold] keep the page.
     */
    fun swipeTarget(step: Int, totalDrag: Float, threshold: Float, rtl: Boolean = false): Int {
        val current = clamp(step)
        if (!totalDrag.isFinite() || !threshold.isFinite()) return current
        val travel = if (rtl) -totalDrag else totalDrag
        return when {
            travel <= -threshold -> next(current)
            travel >= threshold -> previous(current)
            else -> current
        }
    }
}

private const val PageMillis = 280

/** "catppuccin-mocha" -> "Catppuccin Mocha"; the ytui brand keeps its lower-case name. */
internal fun themeLabel(id: String): String =
    id.split('-', '_', ' ')
        .filter { it.isNotEmpty() }
        .joinToString(" ") { word ->
            if (word.equals("ytui", ignoreCase = true)) "ytui"
            else word.replaceFirstChar(Char::titlecase)
        }
        .ifEmpty { id }

/** Full-screen first-run setup. Calls [onFinish] on skip and on completion; callers persist. */
@Composable
internal fun Onboarding(vm: YtuiViewModel, dark: Boolean, onFinish: () -> Unit) {
    val reduced = LocalReducedMotion.current
    val mobile by vm.mobile.collectAsState()
    var step by rememberSaveable { mutableIntStateOf(0) }
    val colors = MaterialTheme.colorScheme
    // Palette changes apply live while choosing a theme; ease the whole overlay between them.
    val background by colorTween(colors.background, reduced)
    val content by colorTween(colors.onBackground, reduced)
    val rtl = LocalLayoutDirection.current == LayoutDirection.Rtl
    val gestures = mobile.optBoolean("gestures", true)

    BackHandler {
        val target = OnboardingSteps.backTarget(step)
        if (target == null) onFinish() else step = target
    }
    CompositionLocalProvider(LocalContentColor provides content) {
        Column(
            Modifier.fillMaxSize()
                .background(background)
                .safeDrawingPadding()
                .semantics { paneTitle = "Setup" }
                .testTag("onboarding")
                .pointerInput(step, gestures, rtl) {
                    if (!gestures) return@pointerInput
                    var total = 0f
                    val threshold = 80.dp.toPx()
                    // Only claims the gesture after touch slop, so taps on switches/chips and
                    // vertical scrolling are untouched.
                    detectHorizontalDragGestures(
                        onDragStart = { total = 0f },
                        onDragCancel = { total = 0f },
                        onDragEnd = { step = OnboardingSteps.swipeTarget(step, total, threshold, rtl) },
                    ) { _, amount ->
                        total += amount
                    }
                }
        ) {
            TopBar(step, background, onBack = { step = OnboardingSteps.previous(step) }, onFinish)
            AnimatedContent(
                targetState = step,
                modifier = Modifier.weight(1f).fillMaxWidth(),
                transitionSpec = {
                    if (reduced) EnterTransition.None togetherWith ExitTransition.None using null
                    else {
                        val forward = OnboardingSteps.isForward(initialState, targetState)
                        val sign = if (forward) 1 else -1
                        (slideInHorizontally(tween(PageMillis, easing = FastOutSlowInEasing)) {
                            it / 4 * sign
                        } + fadeIn(tween(PageMillis, easing = FastOutSlowInEasing)))
                            .togetherWith(
                                slideOutHorizontally(
                                    tween(PageMillis, easing = FastOutSlowInEasing)
                                ) {
                                    -it / 4 * sign
                                } + fadeOut(tween(PageMillis * 2 / 3, easing = FastOutSlowInEasing))
                            )
                    }
                },
                label = "onboarding-page",
            ) { page ->
                when (page) {
                    OnboardingSteps.WELCOME -> WelcomePage(background)
                    OnboardingSteps.LOOK -> LookPage(vm, dark)
                    OnboardingSteps.PLAYBACK -> PlaybackPage(vm)
                    OnboardingSteps.ACCOUNT -> AccountPage(vm)
                    else -> ReadyPage(vm)
                }
            }
            BottomBar(step, vm, onNext = { step = OnboardingSteps.next(step) }, onFinish)
        }
    }
}

@Composable
private fun colorTween(target: Color, reduced: Boolean) =
    animateColorAsState(
        target,
        if (reduced) snap<Color>() else tween<Color>(300),
        label = "onboarding-colour",
    )

// ---------------------------------------------------------------- chrome

@Composable
private fun TopBar(step: Int, background: Color, onBack: () -> Unit, onFinish: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.width(72.dp), contentAlignment = Alignment.CenterStart) {
            if (step > 0) MediaIconButton(Glyph.Back, "Back", "onboarding-back", onBack)
        }
        Box(Modifier.weight(1f), contentAlignment = Alignment.Center) { StepDots(step) }
        Box(Modifier.width(72.dp), contentAlignment = Alignment.CenterEnd) {
            if (OnboardingSteps.showSkip(step))
                TextButton(onFinish, Modifier.heightIn(min = 48.dp).testTag("onboarding-skip")) {
                    Text(
                        "Skip",
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
        }
    }
}

@Composable
private fun StepDots(step: Int) {
    val reduced = LocalReducedMotion.current
    val colors = MaterialTheme.colorScheme
    Row(
        Modifier.semantics(mergeDescendants = true) {
                contentDescription = "Setup progress"
                stateDescription = OnboardingSteps.stateDescription(step)
                liveRegion = LiveRegionMode.Polite
            }
            .testTag("onboarding-steps"),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        for (index in 0 until OnboardingSteps.COUNT) {
            val width by
                animateDpAsState(
                    if (index == step) 24.dp else 8.dp,
                    if (reduced) snap<Dp>() else spring(Spring.DampingRatioMediumBouncy),
                    label = "dot-width",
                )
            val color by
                colorTween(
                    when {
                        index == step -> colors.onBackground
                        index < step -> colors.onSurfaceVariant
                        else -> colors.outlineVariant
                    },
                    reduced,
                )
            Box(Modifier.size(width, 8.dp).clip(CircleShape).background(color))
        }
    }
}

@Composable
private fun BottomBar(step: Int, vm: YtuiViewModel, onNext: () -> Unit, onFinish: () -> Unit) {
    val account by vm.account.collectAsState()
    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        Box(
            Modifier.widthIn(max = 560.dp).fillMaxWidth().padding(horizontal = 24.dp, vertical = 16.dp),
            contentAlignment = Alignment.Center,
        ) {
            when {
                OnboardingSteps.isLast(step) -> Action("Start watching", "onboarding-finish", true, onFinish)
                step == OnboardingSteps.WELCOME -> Action("Get started", "onboarding-next", true, onNext)
                // Not signed in: sign-in is the page's primary action, so advancing is quiet.
                step == OnboardingSteps.ACCOUNT && !account.optBoolean("connected") ->
                    QuietAction("Maybe later", Modifier.testTag("onboarding-next"), action = onNext)
                else -> Action("Continue", "onboarding-next", true, onNext)
            }
        }
    }
}

// ---------------------------------------------------------------- shared page pieces

/** Fades and slides a block up after [index] * 70ms; instant under reduced motion. */
@Composable
private fun Staggered(index: Int, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    val reduced = LocalReducedMotion.current
    var shown by remember { mutableStateOf(reduced) }
    LaunchedEffect(Unit) {
        if (!shown) {
            delay(80L + index * 70L)
            shown = true
        }
    }
    val progress by
        animateFloatAsState(
            if (shown) 1f else 0f,
            if (reduced) snap<Float>() else tween<Float>(360, easing = FastOutSlowInEasing),
            label = "stagger",
        )
    Box(
        modifier.graphicsLayer {
            alpha = progress
            translationY = (1f - progress) * 20.dp.toPx()
        }
    ) {
        content()
    }
}

/**
 * Caps width at 560dp and scrolls when the page is taller than the viewport. Centred
 * pages (welcome, ready) sit in the middle; setup pages start at a fixed offset so the
 * icon and heading do not jump between steps of different heights.
 */
@Composable
private fun PageColumn(
    horizontal: Alignment.Horizontal = Alignment.Start,
    content: @Composable () -> Unit,
) {
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val minimum = maxHeight
        val centred = horizontal == Alignment.CenterHorizontally
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Column(
                Modifier.heightIn(min = minimum)
                    .widthIn(max = 560.dp)
                    .fillMaxWidth()
                    .padding(
                        start = 24.dp,
                        end = 24.dp,
                        top = if (centred) 16.dp else (minimum * 0.08f).coerceIn(16.dp, 72.dp),
                        bottom = 16.dp,
                    ),
                horizontalAlignment = horizontal,
                verticalArrangement =
                    Arrangement.spacedBy(
                        16.dp,
                        if (centred) Alignment.CenterVertically else Alignment.Top,
                    ),
            ) {
                content()
            }
        }
    }
}

@Composable
private fun PageTitle(title: String, body: String, center: Boolean = false, firstIndex: Int = 1) {
    val align = if (center) TextAlign.Center else TextAlign.Start
    Column(
        Modifier.fillMaxWidth(),
        horizontalAlignment = if (center) Alignment.CenterHorizontally else Alignment.Start,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Staggered(firstIndex) {
            Text(
                title,
                Modifier.semantics { heading() },
                style = MaterialTheme.typography.headlineMedium,
                fontWeight = FontWeight.Bold,
                textAlign = align,
            )
        }
        Staggered(firstIndex + 1) {
            Text(
                body,
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = align,
            )
        }
    }
}

/** Small tinted badge shown above a page title. */
@Composable
private fun PageBadge(glyph: Glyph) {
    val brand = mediaBrand(MaterialTheme.colorScheme.background)
    Staggered(0) {
        Box(
            Modifier.size(56.dp).background(brand.copy(alpha = 0.16f), CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            MediaIcon(glyph, Modifier.size(28.dp), brand)
        }
    }
}

@Composable
private fun SettingSwitchRow(
    glyph: Glyph,
    title: String,
    subtitle: String,
    checked: Boolean,
    tag: String,
    enabled: Boolean = true,
    onChange: (Boolean) -> Unit,
) {
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = 64.dp)
            .clip(RoundedCornerShape(16.dp))
            .background(MaterialTheme.colorScheme.surfaceContainer)
            .toggleable(checked, enabled = enabled, role = Role.Switch, onValueChange = onChange)
            .padding(horizontal = 16.dp, vertical = 10.dp)
            .testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        MediaIcon(glyph, Modifier.size(24.dp), MaterialTheme.colorScheme.onSurfaceVariant)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            Text(
                subtitle,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Switch(checked, null, enabled = enabled)
    }
}

// ---------------------------------------------------------------- 1 Welcome

@Composable
private fun WelcomePage(background: Color) {
    BoxWithConstraints(Modifier.fillMaxSize()) {
        if (maxWidth >= 840.dp) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Row(
                    Modifier.widthIn(max = 960.dp).fillMaxWidth().padding(horizontal = 48.dp),
                    horizontalArrangement = Arrangement.spacedBy(48.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    WelcomeHero(background, 260.dp)
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        WelcomeText(center = false)
                    }
                }
            }
        } else {
            PageColumn(Alignment.CenterHorizontally) {
                WelcomeHero(background, 240.dp)
                WelcomeText(center = true)
            }
        }
    }
}

@Composable
private fun WelcomeText(center: Boolean) {
    val align = if (center) TextAlign.Center else TextAlign.Start
    Column(
        Modifier.fillMaxWidth(),
        horizontalAlignment = if (center) Alignment.CenterHorizontally else Alignment.Start,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Staggered(3) {
            Text(
                "ytui",
                style = MaterialTheme.typography.displaySmall,
                fontWeight = FontWeight.Bold,
                textAlign = align,
            )
        }
        Staggered(4) {
            Text(
                "YouTube, YouTube Music and radio — native, fast, yours.",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = align,
            )
        }
    }
}

@Composable
private fun WelcomeHero(background: Color, size: Dp) {
    val reduced = LocalReducedMotion.current
    val brand = mediaBrand(background)
    val pop = remember { Animatable(if (reduced) 1f else 0.4f) }
    LaunchedEffect(reduced) {
        if (reduced) pop.snapTo(1f)
        else pop.animateTo(1f, spring(Spring.DampingRatioMediumBouncy, Spring.StiffnessLow))
    }
    Box(Modifier.size(size), contentAlignment = Alignment.Center) {
        PulseRings(brand, reduced)
        Box(
            Modifier.size(size * 0.46f)
                .graphicsLayer {
                    scaleX = pop.value
                    scaleY = pop.value
                    alpha = ((pop.value - 0.4f) / 0.6f).coerceIn(0f, 1f)
                }
                .background(brand.copy(alpha = 0.18f), CircleShape)
                .border(BorderStroke(2.dp, brand.copy(alpha = 0.6f)), CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            MediaIcon(Glyph.Play, Modifier.size(size * 0.22f), brand)
        }
    }
}

/** Three soft rings expanding outward; a still halo when motion is reduced. */
@Composable
private fun PulseRings(color: Color, reduced: Boolean) {
    if (reduced) {
        Canvas(Modifier.fillMaxSize()) {
            drawCircle(color.copy(alpha = 0.12f), size.minDimension * 0.36f, style = Stroke(2.dp.toPx()))
        }
        return
    }
    val transition = rememberInfiniteTransition(label = "rings")
    val phase by
        transition.animateFloat(
            0f,
            1f,
            infiniteRepeatable(tween(3600, easing = LinearEasing), RepeatMode.Restart),
            label = "ring-phase",
        )
    Canvas(Modifier.fillMaxSize()) {
        val inner = size.minDimension * 0.23f
        val outer = size.minDimension * 0.5f
        for (ring in 0 until 3) {
            val p = (phase + ring / 3f) % 1f
            val radius = inner + (outer - inner) * FastOutSlowInEasing.transform(p)
            drawCircle(color.copy(alpha = 0.32f * (1f - p)), radius, style = Stroke(2.dp.toPx()))
        }
    }
}

// ---------------------------------------------------------------- 2 Look & feel

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun LookPage(vm: YtuiViewModel, dark: Boolean) {
    val config by vm.config.collectAsState()
    val themes by vm.themes.collectAsState()
    val ready by vm.ready.collectAsState()
    // Saving writes the whole settings document. Until the user's own settings have
    // loaded (or if loading failed), editing here could replace them with defaults.
    val editable = ready && config.length() > 0
    val selected = config.optString("theme", "system")
    val choices = listOf("system") + themes.filter { it != "system" }
    PageColumn {
        PageBadge(Glyph.Palette)
        PageTitle("Look & feel", "Pick a colour theme. It applies right away and you can change it any time in Settings.")
        if (!editable)
            Text(
                if (ready) "Your settings could not be loaded, so they are left unchanged. Choose a theme later in Settings."
                else "Loading your settings…",
                Modifier.testTag("onboarding-settings-pending"),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        Staggered(3, Modifier.fillMaxWidth()) {
            FlowRow(
                Modifier.fillMaxWidth().selectableGroup(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                for (name in choices)
                    ThemeChip(name, name == selected, editable) {
                        vm.updateConfig(dark) { it.put("theme", name) }
                    }
            }
        }
        Staggered(4, Modifier.fillMaxWidth()) {
            SettingSwitchRow(
                Glyph.Speed,
                "Reduce motion",
                "Fewer animations and still visualizers",
                config.optBoolean("reduced_motion"),
                "onboarding-reduce-motion",
                enabled = editable,
            ) { value ->
                vm.updateConfig(dark) { it.put("reduced_motion", value) }
            }
        }
    }
}

@Composable
private fun ThemeChip(name: String, selected: Boolean, enabled: Boolean, onClick: () -> Unit) {
    val reduced = LocalReducedMotion.current
    val colors = MaterialTheme.colorScheme
    val container by colorTween(if (selected) colors.onBackground else colors.surfaceContainerHigh, reduced)
    val label by colorTween(if (selected) colors.background else colors.onSurface, reduced)
    val outline by colorTween(if (selected) colors.onBackground else colors.outlineVariant, reduced)
    Row(
        Modifier.heightIn(min = 48.dp)
            .clip(MediaTokens.pill)
            .background(container)
            .border(BorderStroke(1.dp, outline), MediaTokens.pill)
            .selectable(selected, enabled = enabled, role = Role.RadioButton, onClick = onClick)
            .padding(horizontal = 18.dp)
            .testTag("onboarding-theme-$name"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        AnimatedVisibility(
            selected,
            enter =
                if (reduced) EnterTransition.None
                else
                    expandHorizontally(spring(Spring.DampingRatioMediumBouncy)) +
                        scaleIn(spring(Spring.DampingRatioMediumBouncy)) +
                        fadeIn(),
            exit =
                if (reduced) ExitTransition.None
                else shrinkHorizontally(tween(160)) + fadeOut(tween(120)),
        ) {
            Row {
                MediaIcon(Glyph.Check, Modifier.size(18.dp), label)
                Spacer(Modifier.width(8.dp))
            }
        }
        Text(
            themeLabel(name),
            style = MaterialTheme.typography.labelLarge,
            color = label,
        )
    }
}

// ---------------------------------------------------------------- 3 Playback

@Composable
private fun PlaybackPage(vm: YtuiViewModel) {
    val mobile by vm.mobile.collectAsState()
    PageColumn {
        PageBadge(Glyph.Play)
        PageTitle("Playback", "Choose how ytui plays. Every option is also in Settings later.")
        val rows =
            listOf(
                Triple("background_playback", Glyph.Play, "Keep playing in the background" to "Audio continues when you leave the app"),
                Triple("autoplay_related", Glyph.Next, "Autoplay related videos" to "Keep going when the queue ends"),
                Triple("preload_next", Glyph.Download, "Preload the next item" to "Start the next video sooner"),
                Triple("gestures", Glyph.Expand, "Swipe gestures" to "Swipe to seek and move between pages"),
            )
        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            rows.forEachIndexed { index, (key, glyph, text) ->
                Staggered(3 + index, Modifier.fillMaxWidth()) {
                    SettingSwitchRow(
                        glyph,
                        text.first,
                        text.second,
                        mobile.optBoolean(key, true),
                        "onboarding-toggle-$key",
                    ) { value ->
                        vm.updateMobile(key, value)
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- 4 Account

@Composable
private fun AccountPage(vm: YtuiViewModel) {
    val reduced = LocalReducedMotion.current
    // The sign-in activity closes back to this page; the flow updates by itself.
    val account by vm.account.collectAsState()
    val connected = account.optBoolean("connected")
    PageColumn {
        PageBadge(Glyph.Account)
        PageTitle(
            "Your account",
            "Signing in is optional. It unlocks your subscriptions, playlists and music library. " +
                "Your session stays encrypted on this device.",
        )
        Staggered(3, Modifier.fillMaxWidth()) {
            AnimatedContent(
                connected,
                transitionSpec = {
                    if (reduced) EnterTransition.None togetherWith ExitTransition.None using null
                    else fadeIn(tween(220)) togetherWith fadeOut(tween(120))
                },
                label = "account-state",
            ) { signedIn ->
                if (signedIn) {
                    val brand = mediaBrand(MaterialTheme.colorScheme.background)
                    Row(
                        Modifier.fillMaxWidth()
                            .heightIn(min = 64.dp)
                            .clip(RoundedCornerShape(16.dp))
                            .background(MaterialTheme.colorScheme.surfaceContainer)
                            .padding(horizontal = 16.dp, vertical = 10.dp)
                            .testTag("onboarding-signed-in"),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(16.dp),
                    ) {
                        MediaIcon(Glyph.Check, Modifier.size(24.dp), brand)
                        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                            Text("Signed in", style = MaterialTheme.typography.titleMedium)
                            Text(
                                "Your library is ready.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                } else {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Action("Sign in to YouTube", "onboarding-sign-in") {
                            vm.actionError.value = null
                            vm.effect("login")
                        }
                        // The sign-in card's "import instead" link returns here, so offer it.
                        QuietAction("Import a cookie file", Modifier.testTag("onboarding-import")) {
                            vm.actionError.value = null
                            vm.effect("pick", "kind" to "cookies")
                        }
                    }
                }
            }
        }
        // Errors (e.g. sign-in unavailable behind an explicit proxy) would otherwise be
        // reported only by the app hidden beneath setup.
        val error by vm.actionError.collectAsState()
        error?.let {
            Text(
                it,
                Modifier.semantics { liveRegion = LiveRegionMode.Polite }
                    .testTag("onboarding-error"),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.error,
            )
        }
    }
}

// ---------------------------------------------------------------- 5 Ready

@Composable
private fun ReadyPage(vm: YtuiViewModel) {
    val config by vm.config.collectAsState()
    val mobile by vm.mobile.collectAsState()
    val account by vm.account.collectAsState()
    PageColumn(Alignment.CenterHorizontally) {
        AnimatedCheck()
        PageTitle("You're all set", "Here's what you chose. Everything lives in Settings if you change your mind.", center = true, firstIndex = 2)
        Staggered(4, Modifier.fillMaxWidth()) {
            Column(
                Modifier.fillMaxWidth()
                    .clip(RoundedCornerShape(16.dp))
                    .background(MaterialTheme.colorScheme.surfaceContainer)
                    .padding(horizontal = 16.dp, vertical = 8.dp)
                    .testTag("onboarding-summary")
            ) {
                SummaryRow("Theme", themeLabel(config.optString("theme", "system")))
                SummaryRow("Reduce motion", onOff(config.optBoolean("reduced_motion")))
                SummaryRow("Background playback", onOff(mobile.optBoolean("background_playback", true)))
                SummaryRow("Autoplay related", onOff(mobile.optBoolean("autoplay_related", true)))
                SummaryRow("Account", if (account.optBoolean("connected")) "Signed in" else "Not signed in")
            }
        }
    }
}

private fun onOff(value: Boolean) = if (value) "On" else "Off"

@Composable
private fun SummaryRow(label: String, value: String) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 40.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(value, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold)
    }
}

/** A circle that pops in, then a check mark that draws itself. */
@Composable
private fun AnimatedCheck() {
    val reduced = LocalReducedMotion.current
    val brand = mediaBrand(MaterialTheme.colorScheme.background)
    val pop = remember { Animatable(if (reduced) 1f else 0f) }
    val draw = remember { Animatable(if (reduced) 1f else 0f) }
    LaunchedEffect(reduced) {
        if (reduced) {
            pop.snapTo(1f)
            draw.snapTo(1f)
        } else {
            delay(120)
            pop.animateTo(1f, spring(Spring.DampingRatioMediumBouncy, Spring.StiffnessLow))
            draw.animateTo(1f, tween(450, easing = FastOutSlowInEasing))
        }
    }
    Canvas(
        Modifier.size(120.dp)
            .graphicsLayer {
                scaleX = pop.value
                scaleY = pop.value
                alpha = pop.value.coerceIn(0f, 1f)
            }
            .semantics { contentDescription = "Setup complete" }
            .testTag("onboarding-check")
    ) {
        val s = size.minDimension
        drawCircle(brand.copy(alpha = 0.18f), s / 2f)
        drawCircle(brand.copy(alpha = 0.6f), s / 2f - 1.dp.toPx(), style = Stroke(2.dp.toPx()))
        val check =
            Path().apply {
                moveTo(s * 0.30f, s * 0.52f)
                lineTo(s * 0.45f, s * 0.67f)
                lineTo(s * 0.71f, s * 0.36f)
            }
        val measure = PathMeasure().apply { setPath(check, false) }
        val partial = Path()
        measure.getSegment(0f, measure.length * draw.value, partial, true)
        drawPath(
            partial,
            brand,
            style = Stroke(max(4.dp.toPx(), 1f), cap = StrokeCap.Round, join = StrokeJoin.Round),
        )
    }
}
