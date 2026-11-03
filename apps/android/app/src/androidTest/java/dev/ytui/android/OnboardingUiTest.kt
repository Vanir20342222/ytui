package dev.ytui.android

import android.content.Intent
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.*
import org.junit.runner.RunWith

/**
 * First-run setup on a disposable offline launch. Onboarding is only shown because the
 * launch passes ytui.onboarding; the stored completion is restored in @After.
 *
 * While setup is visible ComposeHost clears the semantics of the app underneath, so these
 * checks wait for the onboarding tags rather than NativeUiTest's app-background marker.
 */
@RunWith(AndroidJUnit4::class)
@OptIn(androidx.compose.ui.test.ExperimentalTestApi::class)
class OnboardingUiTest {
    @get:Rule
    val compose = createEmptyComposeRule(effectContext = kotlinx.coroutines.Dispatchers.Main)
    @get:Rule
    val notifications =
        androidx.test.rule.GrantPermissionRule.grant(
            *if (android.os.Build.VERSION.SDK_INT >= 33)
                arrayOf("android.permission.POST_NOTIFICATIONS")
            else emptyArray()
        )
    private lateinit var scenario: ActivityScenario<RustActivity>
    private val context
        get() = InstrumentationRegistry.getInstrumentation().targetContext

    @Before
    fun launch() {
        OnboardingStore.reset(context)
        val intent =
            Intent(context, RustActivity::class.java)
                .putExtra("ytui.offline-ui", true)
                .putExtra("ytui.onboarding", true)
        scenario = ActivityScenario.launch(intent)
        awaitStep("Step 1 of 5")
    }

    @After
    fun restoreSetupState() {
        try {
            if (::scenario.isInitialized) scenario.close()
        } finally {
            OnboardingStore.complete(context)
        }
    }

    @Test
    fun skipOnTheFirstPageDismissesSetupAndRecordsCompletion() {
        compose.onNodeWithTag("onboarding-skip").assertIsDisplayed().performClick()
        awaitTag("onboarding", present = false)
        Assert.assertFalse(OnboardingStore.shouldShow(context))
    }

    @Test
    fun nextStepsThroughEveryPageUntilFinish() {
        var moves = 0
        while (!exists("onboarding-finish")) {
            Assert.assertTrue("Setup must reach its finish page within five pages", moves < 4)
            compose.onNodeWithTag("onboarding-skip").assertIsDisplayed()
            compose.onNodeWithTag("onboarding-next").performClick()
            moves++
            awaitStep("Step ${moves + 1} of 5")
        }
        Assert.assertEquals("Five pages means four forward moves", 4, moves)
        Assert.assertFalse(
            "The finish page hides Skip",
            exists("onboarding-skip"),
        )
        compose.onNodeWithTag("onboarding-finish").performClick()
        awaitTag("onboarding", present = false)
        Assert.assertFalse(OnboardingStore.shouldShow(context))
    }

    @Test
    fun backReturnsToThePreviousPageAndTheFirstPageHasNoBack() {
        compose.onNodeWithTag("onboarding-back").assertDoesNotExist()
        compose.onNodeWithTag("onboarding-next").performClick()
        awaitStep("Step 2 of 5")
        compose.onNodeWithTag("onboarding-next").performClick()
        awaitStep("Step 3 of 5")
        compose.onNodeWithTag("onboarding-back").performClick()
        awaitStep("Step 2 of 5")
        compose.onNodeWithTag("onboarding-back").performClick()
        awaitStep("Step 1 of 5")
        compose.onNodeWithTag("onboarding-back").assertDoesNotExist()
        compose.onNodeWithTag("onboarding-skip").assertIsDisplayed()
    }

    private fun exists(tag: String): Boolean =
        compose.onAllNodesWithTag(tag).fetchSemanticsNodes(atLeastOneRootRequired = false).isNotEmpty()

    private fun awaitTag(tag: String, present: Boolean) {
        compose.waitUntil(10_000) { exists(tag) == present }
    }

    private fun awaitStep(description: String) {
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasTestTag("onboarding-steps") and
                        SemanticsMatcher.expectValue(
                            SemanticsProperties.StateDescription,
                            description,
                        )
                )
                .fetchSemanticsNodes(atLeastOneRootRequired = false)
                .isNotEmpty()
        }
    }
}
