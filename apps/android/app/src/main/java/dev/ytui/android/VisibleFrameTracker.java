package dev.ytui.android;

import android.view.View;
import androidx.media3.common.Player;
import androidx.media3.ui.PlayerView;

/** Count a rendered frame only while its native player view is actually visible. */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
final class VisibleFrameTracker implements Player.Listener {
  private final PlayerView view;
  private Player player;
  private boolean presentable = true;

  void setPresentable(boolean next) {
    boolean changed = presentable != next;
    presentable = next;
    PlaybackService service = PlaybackService.current;
    if (changed && next && service != null && service.firstFrameMs > 0) onRenderedFirstFrame();
  }

  VisibleFrameTracker(PlayerView view) {
    this.view = view;
  }

  void bind(Player next) {
    if (player == next) return;
    if (player != null) player.removeListener(this);
    player = next;
    view.setPlayer(next);
    if (player != null) player.addListener(this);
  }

  @Override
  public void onRenderedFirstFrame() {
    PlaybackService service = PlaybackService.current;
    if (service == null || !presentable) return;
    final long generation = service.generation, timing = service.startupToken;
    // A codec's release event can precede presentation. Keep the conservative
    // UI-frame guard for both surface types without blocking the UI.
    view.postOnAnimation(
        () ->
            view.postOnAnimation(
                () -> {
                  if (PlaybackService.current == service
                      && service.generation == generation
                      && service.startupToken == timing
                      && presentable
                      && player == service.controlsPlayer
                      && view.getPlayer() == player
                      && view.isShown()
                      && view.getWindowVisibility() == View.VISIBLE
                      && view.getWidth() > 0
                      && view.getHeight() > 0) service.visibleFrame();
                }));
  }
}
