package dev.ytui.android;

import android.os.Bundle;
import androidx.activity.ComponentActivity;
import androidx.activity.OnBackPressedCallback;
import androidx.core.view.*;
import androidx.lifecycle.Lifecycle;
import androidx.media3.ui.PlayerView;

/** One shared decoder/session: fullscreen never extracts or prepares again. */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
public final class FullscreenActivity extends ComponentActivity {
  static volatile boolean visible;
  private PlayerView view;
  private VisibleFrameTracker frames;

  @Override
  protected void onCreate(Bundle state) {
    super.onCreate(state);
    PlaybackService service = PlaybackService.current;
    if (service == null) {
      finish();
      return;
    }
    visible = true;
    getOnBackPressedDispatcher()
        .addCallback(
            this,
            new OnBackPressedCallback(true) {
              @Override
              public void handleOnBackPressed() {
                finish();
              }
            });
    ComposeHost.fullscreenChanged(true);
    WindowCompat.setDecorFitsSystemWindows(getWindow(), false);
    WindowInsetsControllerCompat bars =
        WindowCompat.getInsetsController(getWindow(), getWindow().getDecorView());
    bars.hide(WindowInsetsCompat.Type.systemBars());
    bars.setSystemBarsBehavior(WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
    getWindow().addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
    view = (PlayerView) getLayoutInflater().inflate(R.layout.ytui_player, null);
    view.setBackgroundColor(android.graphics.Color.BLACK);
    view.setUseController(true);
    PlayerChrome.apply(view);
    view.findViewById(R.id.ytui_collapse).setOnClickListener(button -> finish());
    view.setControllerAnimationEnabled(
        !getIntent().getBooleanExtra("reduced_motion", false)
            && android.animation.ValueAnimator.areAnimatorsEnabled());
    view.setShowSubtitleButton(true);
    view.setShowShuffleButton(true);
    view.setFullscreenButtonState(true);
    view.setFullscreenButtonClickListener(
        fullscreen -> {
          if (!fullscreen) finish();
        });
    frames = new VisibleFrameTracker(view);
    VideoGestures.attach(view);
    setContentView(view);
  }

  @Override
  protected void onResume() {
    super.onResume();
    visible = true;
    MediaBridge.uiVisible = true;
    PlaybackService service = PlaybackService.current;
    if (view != null && service != null) frames.bind(service.controlsPlayer);
    MediaBridge.notifyUi();
  }

  @Override
  protected void onPause() {
    if (view != null) frames.bind(null);
    super.onPause();
  }

  @Override
  protected void onStop() {
    visible = false;
    RustActivity activity = RustActivity.current();
    if (activity == null
        || !activity.getLifecycle().getCurrentState().isAtLeast(Lifecycle.State.STARTED)) {
      MediaBridge.uiVisible = false;
      if (!isChangingConfigurations() && PlaybackService.current != null)
        PlaybackService.current.pauseForBackground();
    }
    super.onStop();
  }

  @Override
  protected void onDestroy() {
    visible = false;
    if (view != null) frames.bind(null);
    ComposeHost.fullscreenChanged(false);
    super.onDestroy();
  }
}
