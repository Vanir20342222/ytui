package dev.ytui.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class OnboardingStepsTest {
    @Test
    fun navigationIsClampedToTheFivePages() {
        assertEquals(5, OnboardingSteps.COUNT)
        assertEquals(1, OnboardingSteps.next(0))
        assertEquals(4, OnboardingSteps.next(4))
        assertEquals(0, OnboardingSteps.previous(0))
        assertEquals(2, OnboardingSteps.previous(3))
        assertEquals(0, OnboardingSteps.clamp(-7))
        assertEquals(4, OnboardingSteps.clamp(99))
    }

    @Test
    fun backLeavesSetupOnlyFromTheFirstPage() {
        assertNull(OnboardingSteps.backTarget(0))
        assertEquals(0, OnboardingSteps.backTarget(1))
        assertEquals(3, OnboardingSteps.backTarget(4))
    }

    @Test
    fun skipIsAvailableOnEveryPageExceptTheFinishPage() {
        for (step in 0..3) assertTrue(OnboardingSteps.showSkip(step))
        assertFalse(OnboardingSteps.showSkip(4))
        assertTrue(OnboardingSteps.isLast(4))
        assertFalse(OnboardingSteps.isLast(3))
    }

    @Test
    fun directionAndDescriptionFollowTheStep() {
        assertTrue(OnboardingSteps.isForward(1, 2))
        assertFalse(OnboardingSteps.isForward(2, 1))
        assertEquals("Step 1 of 5", OnboardingSteps.stateDescription(0))
        assertEquals("Step 5 of 5", OnboardingSteps.stateDescription(4))
    }

    @Test
    fun swipesNeedAFullThresholdAndRespectLayoutDirection() {
        assertEquals(2, OnboardingSteps.swipeTarget(1, -120f, 80f))
        assertEquals(0, OnboardingSteps.swipeTarget(1, 120f, 80f))
        assertEquals(1, OnboardingSteps.swipeTarget(1, -79f, 80f))
        assertEquals(0, OnboardingSteps.swipeTarget(0, 300f, 80f))
        assertEquals(4, OnboardingSteps.swipeTarget(4, -300f, 80f))
        assertEquals(0, OnboardingSteps.swipeTarget(1, -120f, 80f, rtl = true))
        assertEquals(2, OnboardingSteps.swipeTarget(1, 120f, 80f, rtl = true))
        assertEquals(1, OnboardingSteps.swipeTarget(1, Float.NaN, 80f))
    }
}
