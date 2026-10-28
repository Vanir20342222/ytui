package dev.ytui.android

import org.junit.Assert.*
import org.junit.Test

class FeedRetryTest {
    @Test
    fun transportFailuresRetryWithoutNeedingAServiceCooldown() {
        val error = RpcError("provider_error", "Temporary connection failure", retryable = true)
        assertEquals(2000L, error.retryDelay(0))
        assertEquals(4000L, error.retryDelay(1))
        assertEquals(8000L, error.retryDelay(2))
        assertNull(error.retryDelay(3))
        assertNull(RpcError("auth_required", "Sign in").retryDelay(0))
    }

    @Test
    fun serviceCooldownIsRespectedAndInvalidAttemptsNeverRetry() {
        val error = RpcError("provider_rejected", "Rate limited", 120_000L, true)
        assertEquals(120_000L, error.retryDelay(0))
        assertEquals(120_000L, error.retryDelay(2))
        assertNull(error.retryDelay(-1))
    }
}
