package dev.ytui.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Pure sign-in state logic only. [AuthUi.host] is excluded because it needs android.net.Uri. */
class AuthUiTest {
    @Test
    fun missingOrEmptyCookieHeadersDoNotLookSignedIn() {
        assertFalse(AuthUi.looksSignedIn(null))
        assertFalse(AuthUi.looksSignedIn(""))
        assertFalse(AuthUi.looksSignedIn("   "))
    }

    @Test
    fun unrelatedCookiesDoNotLookSignedIn() {
        assertFalse(AuthUi.looksSignedIn("VISITOR_INFO1_LIVE=abc; PREF=f6=1"))
        assertFalse(AuthUi.looksSignedIn("__Secure-1PSID=abc; __Secure-3PSID=def"))
    }

    @Test
    fun signedInCookieNamesAreDetectedWhereverTheyAppear() {
        assertTrue(AuthUi.looksSignedIn("SAPISID=x"))
        assertTrue(AuthUi.looksSignedIn("a=1; __Secure-3PAPISID=y"))
        assertTrue(AuthUi.looksSignedIn("LOGIN_INFO=z"))
        assertTrue(AuthUi.looksSignedIn("a=1;SAPISID=x;b=2"))
        assertTrue(AuthUi.looksSignedIn(" LOGIN_INFO = z "))
    }

    @Test
    fun namesThatMerelyContainASignedInNameAreNotDetected() {
        assertFalse(AuthUi.looksSignedIn("XSAPISID=1"))
        assertFalse(AuthUi.looksSignedIn("SAPISIDX=1"))
        assertFalse(AuthUi.looksSignedIn("__Secure-3PAPISID_OLD=1"))
        assertFalse(AuthUi.looksSignedIn("MY_LOGIN_INFO=1"))
    }

    @Test
    fun onlyCookieNamesAreInspectedNotValues() {
        assertFalse(AuthUi.looksSignedIn("foo=SAPISID"))
        assertFalse(AuthUi.looksSignedIn("a=__Secure-3PAPISID; b=LOGIN_INFO"))
    }

    @Test
    fun secureMeansAnHttpsAddress() {
        assertTrue(AuthUi.secure("https://accounts.google.com/signin"))
        assertFalse(AuthUi.secure("http://accounts.google.com/signin"))
        assertFalse(AuthUi.secure(""))
        assertFalse(AuthUi.secure("about:blank"))
    }

    @Test
    fun resetRestoresTheIntroductionState() {
        AuthUi.showWeb(true)
        AuthUi.page("https://accounts.google.com/")
        AuthUi.progress(42)
        AuthUi.sessionFound(true)

        AuthUi.reset(false)

        assertFalse(AuthUi.showingWeb())
        assertEquals("", AuthUi.url.value)
        assertEquals(100, AuthUi.progress.value)
        assertFalse(AuthUi.sessionFound.value)
    }

    @Test
    fun resetCanOpenTheWebStageDirectly() {
        AuthUi.reset(true)

        assertTrue(AuthUi.showingWeb())
        assertEquals("", AuthUi.url.value)
        assertEquals(100, AuthUi.progress.value)
    }

    @Test
    fun showWebTogglesOnlyTheStage() {
        AuthUi.reset(false)
        AuthUi.showWeb(true)
        assertTrue(AuthUi.showingWeb())
        AuthUi.showWeb(false)
        assertFalse(AuthUi.showingWeb())
    }

    @Test
    fun progressIsClampedToAPercentage() {
        AuthUi.progress(42)
        assertEquals(42, AuthUi.progress.value)
        AuthUi.progress(150)
        assertEquals(100, AuthUi.progress.value)
        AuthUi.progress(-3)
        assertEquals(0, AuthUi.progress.value)
        AuthUi.progress(0)
        assertEquals(0, AuthUi.progress.value)
        AuthUi.progress(100)
        assertEquals(100, AuthUi.progress.value)
    }

    @Test
    fun pageAcceptsANullAddressAsEmpty() {
        AuthUi.page("https://accounts.google.com/")
        AuthUi.page(null)
        assertEquals("", AuthUi.url.value)
    }
}
