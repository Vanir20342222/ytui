package dev.ytui.android;

import android.view.View;
import androidx.media3.common.Player;
import androidx.media3.ui.PlayerView;

/** Count a rendered frame only while its native player view is actually visible. */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
final class VisibleFrameTracker implements Player.Listener {
  private final PlayerView view;
  private Player player;

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
    if (service == null) return;
    final long generation = service.generation, timing = service.startupToken;
    // A codec's surface-release event precedes TextureView drawing. Allow a
    // complete UI frame before recording view output, without blocking the UI.
    view.postOnAnimation(
        () ->
            view.postOnAnimation(
                () -> {
                  if (PlaybackService.current == service
                      && service.generation == generation
                      && service.startupToken == timing
                      && player == service.controlsPlayer
                      && view.getPlayer() == player
                      && view.isShown()
                      && view.getWindowVisibility() == View.VISIBLE
                      && view.getWidth() > 0
                      && view.getHeight() > 0) service.visibleFrame();
                }));
  }
}
