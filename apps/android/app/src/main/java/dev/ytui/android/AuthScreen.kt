package dev.ytui.android

import android.view.ViewGroup
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import kotlinx.coroutines.flow.MutableStateFlow

/**
 * State of the explicit account sign-in activity. The layout follows the familiar
 * in-app browser sign-in pattern: a close button and address pill on top, a
 * two-column account card (or the provider page) in the middle and a
 * "you'll come back here" strip at the bottom. Colours stay the user's ytui theme.
 */
internal object AuthUi {
    /** False: native introduction card. True: the provider's page in the WebView. */
    internal val web = MutableStateFlow(false)
    internal val url = MutableStateFlow("")
    internal val progress = MutableStateFlow(100)
    /** A signed-in YouTube cookie name was seen; the user still confirms explicitly. */
    internal val sessionFound = MutableStateFlow(false)
    /** Mirrors the app's reduced-motion setting; the activity has no view model. */
    @Volatile internal var reducedMotion = false
        private set
    @Volatile private var motionKnown = false

    @JvmStatic
    fun motion(reduced: Boolean) {
        reducedMotion = reduced
        motionKnown = true
    }

    /** False after process death restores the sign-in activity without the app UI. */
    @JvmStatic fun motionKnown(): Boolean = motionKnown

    const val HELP_URL = "https://github.com/Vanir20342222/ytui/tree/main/docs/android"
    const val PRIVACY_URL = "https://policies.google.com/privacy"
    const val TERMS_URL = "https://www.youtube.com/t/terms"

    @JvmStatic
    fun reset(showWeb: Boolean) {
        web.value = showWeb
        url.value = ""
        progress.value = 100
        sessionFound.value = false
    }

    @JvmStatic fun showingWeb(): Boolean = web.value

    @JvmStatic
    fun showWeb(value: Boolean) {
        web.value = value
    }

    @JvmStatic
    fun page(address: String?) {
        url.value = address ?: ""
    }

    @JvmStatic
    fun progress(value: Int) {
        progress.value = value.coerceIn(0, 100)
    }

    @JvmStatic
    fun sessionFound(value: Boolean) {
        sessionFound.value = value
    }

    /** Only cookie names are inspected; values are never logged or displayed. */
    @JvmStatic
    fun looksSignedIn(cookieHeader: String?): Boolean =
        cookieHeader
            ?.split(';')
            ?.map { it.substringBefore('=').trim() }
            ?.any { it == "SAPISID" || it == "__Secure-3PAPISID" || it == "LOGIN_INFO" } == true

    /** Host shown in the address pill, e.g. accounts.google.com. */
    @JvmStatic
    fun host(address: String): String =
        runCatching { android.net.Uri.parse(address).host }.getOrNull().orEmpty()

    @JvmStatic fun secure(address: String): Boolean = address.startsWith("https://")
}

@Composable
internal fun AuthScreen(activity: AuthActivity, webView: android.webkit.WebView, busy: Boolean, message: String) {
    val showWeb by AuthUi.web.collectAsState()
    val address by AuthUi.url.collectAsState()
    val progress by AuthUi.progress.collectAsState()
    val found by AuthUi.sessionFound.collectAsState()
    val reduced = AuthUi.reducedMotion
    val scheme = MaterialTheme.colorScheme
    BackHandler {
        when {
            busy -> Unit
            showWeb && webView.canGoBack() -> webView.goBack()
            showWeb -> AuthUi.showWeb(false)
            else -> activity.cancelSession()
        }
    }
    Column(
        Modifier.fillMaxSize()
            .background(scheme.background)
            .safeDrawingPadding()
            .imePadding()
            .testTag("sign-in-screen")
    ) {
        AuthTopBar(
            host = if (showWeb) AuthUi.host(address) else "youtube.com",
            secure = !showWeb || address.isEmpty() || AuthUi.secure(address),
            loading = showWeb && progress < 100,
            progress = progress / 100f,
            close = { if (!busy) activity.cancelSession() },
        )
        Box(Modifier.weight(1f).fillMaxWidth()) {
            AnimatedContent(
                targetState = showWeb,
                transitionSpec = {
                    if (reduced) EnterTransition.None togetherWith ExitTransition.None using null
                    else if (targetState)
                        (fadeIn(tween(220, 80)) +
                            slideInVertically(tween(320, easing = FastOutSlowInEasing)) {
                                it / 10
                            }) togetherWith
                            (fadeOut(tween(160)) + scaleOut(tween(200), targetScale = 0.96f))
                    else
                        (fadeIn(tween(220, 60)) + scaleIn(tween(260), initialScale = 0.96f)) togetherWith
                            (fadeOut(tween(160)) +
                                slideOutVertically(tween(220)) { it / 10 })
                },
                label = "sign-in-stage",
            ) { web ->
                if (web) AuthWebPane(webView)
                else
                    AuthIntro(
                        busy = busy,
                        reduced = reduced,
                        proceed = { activity.beginWeb() },
                        cancel = { if (!busy) activity.cancelSession() },
                        open = activity::openExternal,
                    )
            }
        }
        AuthReturnStrip(
            showWeb = showWeb,
            busy = busy,
            found = found,
            message = message,
            reduced = reduced,
            use = { activity.useSession() },
        )
    }
}

@Composable
private fun AuthTopBar(
    host: String,
    secure: Boolean,
    loading: Boolean,
    progress: Float,
    close: () -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 12.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Surface(shape = CircleShape, color = scheme.surfaceContainerHigh) {
            MediaIconButton(Glyph.Close, "Cancel sign-in", "sign-in-cancel", close)
        }
        Surface(
            Modifier.weight(1f).heightIn(min = 44.dp).testTag("sign-in-address"),
            shape = MediaTokens.pill,
            color = scheme.surfaceContainerHigh,
        ) {
            Box(contentAlignment = Alignment.Center) {
                Row(
                    Modifier.padding(horizontal = 16.dp, vertical = 10.dp).semantics(true) {
                        contentDescription =
                            (if (secure) "Secure page " else "Page ") + host.ifBlank { "loading" }
                    },
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    MediaIcon(
                        if (secure) Glyph.Lock else Glyph.Info,
                        Modifier.size(16.dp),
                        if (secure) scheme.onSurfaceVariant else scheme.error,
                    )
                    Text(
                        host.ifBlank { "Loading…" },
                        style = MaterialTheme.typography.bodyLarge,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                if (loading)
                    LinearProgressIndicator(
                        progress = { progress },
                        modifier =
                            Modifier.align(Alignment.BottomCenter)
                                .fillMaxWidth()
                                .padding(horizontal = 20.dp)
                                .height(2.dp)
                                .clip(MediaTokens.pill),
                        drawStopIndicator = {},
                    )
            }
        }
        // Balances the close button so the address stays visually centred.
        Spacer(Modifier.size(48.dp))
    }
}

@Composable
private fun AuthWebPane(webView: android.webkit.WebView) {
    Surface(
        Modifier.fillMaxSize().padding(horizontal = 12.dp).testTag("sign-in-web"),
        shape = RoundedCornerShape(topStart = 20.dp, topEnd = 20.dp),
        color = MaterialTheme.colorScheme.surface,
    ) {
        AndroidView(
            factory = {
                // The same WebView survives stage changes; detach it from an old host first.
                (webView.parent as? ViewGroup)?.removeView(webView)
                webView
            },
            modifier = Modifier.fillMaxSize(),
        )
    }
}

@Composable
private fun AuthIntro(
    busy: Boolean,
    reduced: Boolean,
    proceed: () -> Unit,
    cancel: () -> Unit,
    open: (String) -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    val entry = remember { Animatable(if (reduced) 1f else 0f) }
    LaunchedEffect(Unit) { if (!reduced) entry.animateTo(1f, tween(420, easing = FastOutSlowInEasing)) }
    BoxWithConstraints(Modifier.fillMaxSize().testTag("sign-in-intro")) {
        val wide = maxWidth >= 720.dp
        val viewport = maxHeight
        Column(
            Modifier.fillMaxSize()
                .verticalScroll(rememberScrollState())
                // Centre the card when it fits; scroll from the top when it does not.
                .heightIn(min = viewport)
                .padding(horizontal = if (wide) 32.dp else 16.dp, vertical = if (wide) 24.dp else 12.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            Column(
                Modifier.widthIn(max = 1040.dp).fillMaxWidth().graphicsLayer {
                    alpha = entry.value
                    translationY = (1f - entry.value) * 24.dp.toPx()
                }
            ) {
                Surface(
                    Modifier.fillMaxWidth().testTag("sign-in-card"),
                    shape = RoundedCornerShape(28.dp),
                    color = scheme.surfaceContainerLow,
                    border = BorderStroke(1.dp, scheme.outlineVariant),
                ) {
                    Column {
                        // Card header: provider mark + "Sign in with Google".
                        Row(
                            Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 16.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            MediaIcon(Glyph.Account, Modifier.size(20.dp), scheme.onSurfaceVariant)
                            Text(
                                "Sign in with Google",
                                style = MaterialTheme.typography.bodyLarge,
                                color = scheme.onSurfaceVariant,
                            )
                        }
                        HorizontalDivider(color = scheme.outlineVariant)
                        if (wide)
                            Row(
                                Modifier.fillMaxWidth().padding(horizontal = 40.dp, vertical = 40.dp),
                                horizontalArrangement = Arrangement.spacedBy(48.dp),
                            ) {
                                AuthIdentity(Modifier.weight(1f), large = true)
                                AuthChoices(Modifier.weight(1f), busy, proceed, cancel, open)
                            }
                        else
                            Column(
                                Modifier.fillMaxWidth().padding(24.dp),
                                verticalArrangement = Arrangement.spacedBy(28.dp),
                            ) {
                                AuthIdentity(Modifier.fillMaxWidth(), large = false)
                                AuthChoices(Modifier.fillMaxWidth(), busy, proceed, cancel, open)
                            }
                    }
                }
                // Below-card footer: privacy note on the left, help links on the right.
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    MediaIcon(Glyph.Lock, Modifier.size(14.dp), scheme.onSurfaceVariant)
                    Text(
                        "Stays on this device",
                        Modifier.padding(start = 6.dp).weight(1f),
                        style = MaterialTheme.typography.bodySmall,
                        color = scheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    for ((label, url) in
                        listOf(
                            "Help" to AuthUi.HELP_URL,
                            "Privacy" to AuthUi.PRIVACY_URL,
                            "Terms" to AuthUi.TERMS_URL,
                        ))
                        TextButton(
                            { open(url) },
                            Modifier.heightIn(min = 48.dp).testTag("sign-in-link-${label.lowercase()}"),
                            colors = ButtonDefaults.textButtonColors(contentColor = scheme.onSurfaceVariant),
                        ) {
                            Text(label, style = MaterialTheme.typography.bodySmall)
                        }
                }
            }
        }
    }
}

/** Left column: app mark, large "Sign in" and "Continue to YouTube". */
@Composable
private fun AuthIdentity(modifier: Modifier, large: Boolean) {
    val scheme = MaterialTheme.colorScheme
    Column(modifier, verticalArrangement = Arrangement.spacedBy(if (large) 20.dp else 12.dp)) {
        Image(
            painterResource(R.drawable.ic_ytui),
            contentDescription = null,
            modifier = Modifier.size(if (large) 56.dp else 48.dp).clip(RoundedCornerShape(14.dp)),
        )
        Text(
            "Sign in",
            Modifier.semantics { heading() },
            style =
                MaterialTheme.typography.displaySmall.copy(
                    fontSize = if (large) 40.sp else 32.sp,
                    lineHeight = if (large) 48.sp else 40.sp,
                    fontWeight = FontWeight.Normal,
                ),
        )
        Text(
            buildAnnotatedString {
                append("Continue to ")
                withStyle(SpanStyle(color = scheme.primary, fontWeight = FontWeight.Medium)) {
                    append("YouTube")
                }
            },
            style = MaterialTheme.typography.bodyLarge,
            color = scheme.onSurfaceVariant,
        )
    }
}

/** Right column: the account choice, an alternative link, a notice and the actions. */
@Composable
private fun AuthChoices(
    modifier: Modifier,
    busy: Boolean,
    proceed: () -> Unit,
    cancel: () -> Unit,
    open: (String) -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    Column(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        // Occupies the place of the provider's identifier field. ytui never asks for
        // a password itself: the real form is Google's own page in the next step.
        Surface(
            onClick = proceed,
            enabled = !busy,
            modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp).testTag("sign-in-account-choice"),
            shape = RoundedCornerShape(8.dp),
            color = scheme.surfaceContainerLow,
            border = BorderStroke(1.dp, scheme.outline),
        ) {
            Row(
                Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                MediaIcon(Glyph.Account, Modifier.size(22.dp), scheme.onSurfaceVariant)
                Column(Modifier.weight(1f)) {
                    Text("Your Google account", style = MaterialTheme.typography.bodyLarge)
                    Text(
                        "Google's sign-in page opens here",
                        style = MaterialTheme.typography.bodySmall,
                        color = scheme.onSurfaceVariant,
                    )
                }
                MediaIcon(Glyph.ArrowRight, Modifier.size(18.dp), scheme.onSurfaceVariant)
            }
        }
        // Like "Forgot email?": the alternative path. Returns to Account, which owns import.
        TextButton(
            cancel,
            Modifier.heightIn(min = 48.dp).testTag("sign-in-import"),
            contentPadding = PaddingValues(horizontal = 0.dp),
            colors = ButtonDefaults.textButtonColors(contentColor = scheme.primary),
        ) {
            Text("Import a cookie file instead?", style = MaterialTheme.typography.labelLarge)
        }
        Spacer(Modifier.height(16.dp))
        val linkStyle =
            TextLinkStyles(SpanStyle(color = scheme.primary, fontWeight = FontWeight.Medium))
        Text(
            buildAnnotatedString {
                append("Before you continue, you can review YouTube's ")
                withLink(LinkAnnotation.Clickable("privacy", linkStyle) { open(AuthUi.PRIVACY_URL) }) {
                    append("Privacy Policy")
                }
                append(" and ")
                withLink(LinkAnnotation.Clickable("terms", linkStyle) { open(AuthUi.TERMS_URL) }) {
                    append("Terms of Service")
                }
                append(
                    ". ytui never sees your password; only the resulting session is kept, encrypted on this device."
                )
            },
            style = MaterialTheme.typography.bodyMedium,
            color = scheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(24.dp))
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            QuietAction("Not now", Modifier.testTag("sign-in-not-now"), enabled = !busy, action = cancel)
            Action("Continue", "sign-in-continue", !busy, proceed)
        }
    }
}

/** Bottom band: app icon and "you'll come back here", plus the explicit confirm action. */
@Composable
private fun AuthReturnStrip(
    showWeb: Boolean,
    busy: Boolean,
    found: Boolean,
    message: String,
    reduced: Boolean,
    use: () -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    val band by
        animateColorAsState(
            if (found && showWeb) lerpColor(scheme.surfaceContainer, scheme.primary, 0.14f)
            else scheme.surfaceContainer,
            tween(if (reduced) 0 else 320),
            label = "sign-in-band",
        )
    Surface(Modifier.fillMaxWidth().testTag("sign-in-return"), color = band) {
        Column {
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth().height(2.dp))
            Row(
                Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 16.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                Image(
                    painterResource(R.drawable.ic_ytui),
                    contentDescription = null,
                    modifier = Modifier.size(32.dp).clip(RoundedCornerShape(8.dp)),
                )
                val text =
                    when {
                        message.isNotBlank() -> message
                        busy -> "Saving your session…"
                        found && showWeb -> "Signed in. Tap Use this session to finish."
                        showWeb -> "You'll come back here once you're signed in"
                        else -> "You'll come back here once you're signed in"
                    }
                AnimatedContent(
                    text,
                    Modifier.weight(1f),
                    transitionSpec = {
                        if (reduced) EnterTransition.None togetherWith ExitTransition.None using null
                        else fadeIn(tween(200)) togetherWith fadeOut(tween(120))
                    },
                    label = "sign-in-return-text",
                ) { value ->
                    Text(
                        value,
                        Modifier.semantics { liveRegion = LiveRegionMode.Polite },
                        style = MaterialTheme.typography.bodyMedium,
                        color = if (message.isNotBlank()) scheme.error else scheme.onSurfaceVariant,
                    )
                }
                AnimatedVisibility(
                    showWeb,
                    enter = if (reduced) EnterTransition.None else fadeIn(tween(200)) + scaleIn(tween(240), 0.9f),
                    exit = if (reduced) ExitTransition.None else fadeOut(tween(120)),
                ) {
                    // Read inside the layer block: the pulse redraws without recomposing.
                    val pulse =
                        if (found && !busy && !reduced)
                            rememberInfiniteTransition(label = "sign-in-pulse").animateFloat(
                                1f,
                                1.05f,
                                infiniteRepeatable(tween(700, easing = FastOutSlowInEasing), RepeatMode.Reverse),
                                label = "sign-in-pulse-scale",
                            )
                        else null
                    Box(
                        Modifier.graphicsLayer {
                                val scale = pulse?.value ?: 1f
                                scaleX = scale
                                scaleY = scale
                            }
                            .semantics {
                                stateDescription = if (found) "Signed-in session detected" else "Waiting for sign-in"
                            }
                    ) {
                        Action("Use this session", "sign-in-complete", !busy, use)
                    }
                }
            }
        }
    }
}

private fun lerpColor(a: androidx.compose.ui.graphics.Color, b: androidx.compose.ui.graphics.Color, t: Float) =
    androidx.compose.ui.graphics.lerp(a, b, t)
