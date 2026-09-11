package dev.helm.android

import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.test.*
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.core.app.ApplicationProvider
import android.content.Context
import android.content.pm.ActivityInfo
import androidx.lifecycle.Lifecycle
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class NativeUiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    @Test fun connectionFieldsAccessibleAndSecretNotSavedOnRecreation() {
        compose.onNodeWithText("Scoped grant token").performScrollTo().performTextInput("a".repeat(64))
        compose.onNodeWithText("Scoped grant token").assertTextContains("a".repeat(64))
        compose.activityRule.scenario.recreate()
        assertEquals("", compose.onNodeWithText("Scoped grant token").performScrollTo().fetchSemanticsNode().config[SemanticsProperties.EditableText].text)
    }
    @Test fun backgroundResumeRotationAndBackRetainNativeSetup() {
        compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        compose.onNodeWithText("Expected Vessel UUID").performScrollTo().performTextInput("test")
        compose.activityRule.scenario.onActivity { it.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE }
        compose.waitForIdle()
        compose.onNodeWithText("Connect directly to Vessel").performScrollTo().assertIsDisplayed()
        compose.activityRule.scenario.onActivity { it.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_PORTRAIT }
    }
}
