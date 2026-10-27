package dev.ytui.android;

import android.app.PendingIntent;
import android.content.Intent;
import android.content.SharedPreferences;
import android.os.*;
import androidx.media3.common.*;
import androidx.media3.datasource.*;
import androidx.media3.exoplayer.*;
import androidx.media3.exoplayer.source.*;
import androidx.media3.session.*;
import java.util.*;
import java.util.concurrent.*;
import org.json.*;

@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
public final class PlaybackService extends MediaSessionService {
  static PlaybackService current;
  ExoPlayer player;
  MediaSession session;
  final ExecutorService worker = Executors.newFixedThreadPool(3);
  final Handler handler = new Handler(Looper.getMainLooper());
  long generation = 0, skipUntil = 0, lastSkipped = -1, lastPositionSave = 0, sleepDeadline = 0;
  String error = "", streamTitle = "", captionText = "";
  JSONObject previewRestore;
  long previewPosition;
  boolean previewPlaying;
  JSONObject currentRequest = new JSONObject();
  JSONArray sponsors = new JSONArray();
  boolean skipSponsors = true, historyEnabled = true, pausedByUser = false;
  int reconnectCount = 0;
  SharedPreferences prefs;

  @Override
  public void onCreate() {
    super.onCreate();
    current = this;
    prefs = getSharedPreferences("player", MODE_PRIVATE);
    player =
        new ExoPlayer.Builder(
                this,
                new DefaultRenderersFactory(this) {
                  @Override
                  protected androidx.media3.exoplayer.audio.AudioSink buildAudioSink(
                      android.content.Context ctx, boolean floating, boolean playbackParams) {
                    return new androidx.media3.exoplayer.audio.DefaultAudioSink.Builder(ctx)
                        .setAudioProcessors(
                            new androidx.media3.common.audio.AudioProcessor[] {new AudioTap()})
                        .setEnableFloatOutput(false)
                        .setEnableAudioTrackPlaybackParams(playbackParams)
                        .build();
                  }
                }.setEnableDecoderFallback(true))
            .build();
    player.setAudioAttributes(
        new AudioAttributes.Builder()
            .setUsage(C.USAGE_MEDIA)
            .setContentType(C.AUDIO_CONTENT_TYPE_MUSIC)
            .build(),
        true);
    player.setHandleAudioBecomingNoisy(true);
    player.setWakeMode(C.WAKE_MODE_NETWORK);
    player.setVolume(prefs.getFloat("volume", 0.7f));
    player.setPlaybackSpeed(prefs.getFloat("speed", 1f));
    session =
        new MediaSession.Builder(this, player)
            .setSessionActivity(
                PendingIntent.getActivity(
                    this,
                    0,
                    new Intent(this, RustActivity.class).setAction("dev.ytui.OPEN_PLAYER"),
                    PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT))
            .build();
    // The Rust UI starts playback directly, so no MediaController bind adds it for us.
    addSession(session);
    player.addListener(
        new Player.Listener() {
          @Override
          public void onCues(androidx.media3.common.text.CueGroup cues) {
            StringBuilder text = new StringBuilder();
            for (androidx.media3.common.text.Cue cue : cues.cues) {
              if (cue.text != null) {
                if (text.length() > 0) text.append("\n");
                text.append(cue.text);
              }
            }
            captionText = text.toString();
            publish();
          }

          @Override
          public void onPlayerError(PlaybackException e) {
            if (isLive() && !rateLimited(e) && reconnectCount < 3) {
              reconnect();
            } else {
              error =
                  rateLimited(e)
                      ? "Service rate limit; wait before retrying"
                      : "Playback failed ("
                          + e.getErrorCodeName()
                          + "). Try another quality or reconnect.";
              publish();
            }
          }

          @Override
          public void onPlaybackStateChanged(int state) {
            if (state == Player.STATE_ENDED) {
              if (isLive()) {
                if (reconnectCount < 3) reconnect();
                else {
                  error = "Live stream disconnected; reconnect limit reached";
                  publish();
                }
              } else if (player.getCurrentPosition() > 0
                  && (player.getDuration() <= 0
                      || player.getCurrentPosition() >= player.getDuration() - 2000)) {
                if (historyEnabled) {
                  JSONObject media = currentRequest.optJSONObject("media");
                  if (media != null)
                    worker.execute(
                        () -> {
                          try {
                            rpc(
                                new JSONObject()
                                    .put("op", "position")
                                    .put("url", media.optString("url"))
                                    .put("milliseconds", 0));
                          } catch (Exception ignored) {
                          }
                        });
                }
                next();
              } else {
                error = "Stream ended before completion; retry the connection";
                publish();
              }
            }
            publish();
          }

          @Override
          public void onIsPlayingChanged(boolean playing) {
            publish();
          }
        });
    worker.execute(
        () -> {
          try {
            JSONObject config = rpc(new JSONObject().put("op", "settings"));
            handler.post(
                () -> {
                  try {
                    command(new JSONObject().put("op", "settings").put("config", config));
                  } catch (Exception ignored) {
                  }
                });
          } catch (Exception ignored) {
          }
        });
    handler.post(ticker);
  }

  @Override
  public MediaSession onGetSession(MediaSession.ControllerInfo controller) {
    return session;
  }

  @Override
  public int onStartCommand(Intent intent, int flags, int id) {
    if (intent != null && "play".equals(intent.getAction())) {
      android.app.NotificationManager notifications =
          getSystemService(android.app.NotificationManager.class);
      notifications.createNotificationChannel(
          new android.app.NotificationChannel(
              "ytui-playback", "ytui playback", android.app.NotificationManager.IMPORTANCE_LOW));
      setMediaNotificationProvider(
          new DefaultMediaNotificationProvider.Builder(this)
              .setNotificationId(4)
              .setChannelId("ytui-playback")
              .build());
      // Android's foreground deadline starts before decoder/buffering completion.
      android.app.Notification preparing =
          new android.app.Notification.Builder(this, "ytui-playback")
              .setSmallIcon(R.drawable.ic_ytui)
              .setContentTitle("ytui")
              .setContentText("Preparing playback")
              .setCategory(android.app.Notification.CATEGORY_TRANSPORT)
              .setOngoing(true)
              .build();
      startForeground(4, preparing);
    }
    int result = super.onStartCommand(intent, flags, id);
    if (intent != null && "play".equals(intent.getAction())) {
      try {
        play(new JSONObject(intent.getStringExtra("request")));
      } catch (Exception e) {
        error = "Invalid playback request";
        publish();
        stopForeground(STOP_FOREGROUND_REMOVE);
        stopSelf();
      }
    }
    return result;
  }

  void play(JSONObject request) throws JSONException {
    if (request.optBoolean("preview")
        && previewRestore == null
        && currentRequest.optJSONObject("media") != null) {
      previewRestore = currentRequest;
      previewPosition = player.getCurrentPosition();
      previewPlaying = player.getPlayWhenReady();
    } else if (!request.optBoolean("preview")) {
      previewRestore = null;
    }
    if (!request.optBoolean("reconnect")) {
      reconnectCount = 0;
      pausedByUser = false;
    }
    generation++;
    currentRequest = request;
    error = "";
    streamTitle = "";
    captionText = "";
    skipUntil = 0;
    lastSkipped = -1;
    sponsors = new JSONArray();
    JSONObject stream = request.getJSONObject("stream"), media = request.getJSONObject("media");
    DefaultHttpDataSource.Factory http =
        new DefaultHttpDataSource.Factory()
            .setConnectTimeoutMs(10000)
            .setReadTimeoutMs(20000)
            .setAllowCrossProtocolRedirects(false)
            .setDefaultRequestProperties(headers(stream.optJSONObject("headers")));
    DataSource.Factory upstream =
        request.optBoolean("offline")
            ? OfflineStore.offlineFactory()
            : new androidx.media3.datasource.DefaultDataSource.Factory(this, http);
    DefaultMediaSourceFactory sources =
        new DefaultMediaSourceFactory(upstream)
            .setLoadErrorHandlingPolicy(
                new androidx.media3.exoplayer.upstream.DefaultLoadErrorHandlingPolicy() {
                  @Override
                  public long getRetryDelayMsFor(
                      androidx.media3.exoplayer.upstream.LoadErrorHandlingPolicy.LoadErrorInfo
                          info) {
                    return rateLimited(info.exception)
                        ? C.TIME_UNSET
                        : super.getRetryDelayMsFor(info);
                  }
                });
    MediaMetadata metadata =
        new MediaMetadata.Builder()
            .setTitle(media.optString("title"))
            .setArtist(media.optString("creator"))
            .build();
    java.util.List<MediaItem.SubtitleConfiguration> captions = new java.util.ArrayList<>();
    JSONObject catalog = stream.optJSONObject("catalog");
    JSONArray choices = catalog == null ? null : catalog.optJSONArray("captions");
    if (choices != null) {
      for (int i = 0; i < Math.min(choices.length(), 40); i++) {
        JSONObject choice = choices.optJSONObject(i);
        if (choice != null)
          captions.add(
              new MediaItem.SubtitleConfiguration.Builder(
                      android.net.Uri.parse(choice.optString("url")))
                  .setMimeType(MimeTypes.TEXT_VTT)
                  .setLabel(choice.optString("label"))
                  .setLanguage(choice.optString("label").split(" ")[0])
                  .build());
      }
    }
    MediaItem main =
        new MediaItem.Builder()
            .setUri(stream.getString("url"))
            .setMediaId(media.optString("url"))
            .setSubtitleConfigurations(captions)
            .setMediaMetadata(metadata)
            .build();
    MediaSource source = sources.createMediaSource(main);
    String audio = stream.optString("audio_url", "");
    if (!audio.isEmpty() && !audio.equals("null"))
      source =
          new MergingMediaSource(true, source, sources.createMediaSource(MediaItem.fromUri(audio)));
    player.setMediaSource(source);
    player.prepare();
    if (pausedByUser) player.pause();
    else player.play();
    if (request.optBoolean("preview")) {
      long version = generation;
      player.setVolume(0f);
      handler.postDelayed(
          () -> {
            if (version == generation) {
              JSONObject restore = previewRestore;
              long position = previewPosition;
              boolean playing = previewPlaying;
              previewRestore = null;
              if (restore != null) {
                try {
                  play(restore);
                  player.seekTo(position);
                  if (!playing) player.pause();
                } catch (Exception error) {
                  player.pause();
                }
              } else {
                player.pause();
                player.setVolume(prefs.getFloat("volume", 0.7f));
              }
            }
          },
          10000);
    } else player.setVolume(prefs.getFloat("volume", 0.7f));
    long version = generation;
    worker.execute(
        () -> {
          try {
            if (request.optBoolean("resume")) {
              JSONObject value =
                  rpc(new JSONObject().put("op", "position").put("url", media.optString("url")));
              long ms = value.optLong("milliseconds");
              handler.post(
                  () -> {
                    if (version == generation) player.seekTo(ms);
                  });
            }
            JSONArray marks = rpcArray(new JSONObject().put("op", "sponsor").put("media", media));
            handler.post(
                () -> {
                  if (version == generation) sponsors = marks;
                });
          } catch (Exception ignored) {
          }
        });
    publish();
  }

  static Map<String, String> headers(JSONObject values) {
    Map<String, String> out = new HashMap<>();
    if (values != null) {
      Iterator<String> names = values.keys();
      while (names.hasNext()) {
        String name = names.next();
        if (Arrays.asList("user-agent", "referer", "origin", "accept", "accept-language")
            .contains(name.toLowerCase(Locale.ROOT))) out.put(name, values.optString(name));
      }
    }
    return out;
  }

  static JSONObject rpc(JSONObject data) throws Exception {
    JSONObject response = new JSONObject(NativeBridge.request(data.toString()));
    if (!response.optBoolean("ok")) throw new Exception(response.optString("error"));
    return response.optJSONObject("data") == null
        ? new JSONObject()
        : response.getJSONObject("data");
  }

  static JSONArray rpcArray(JSONObject data) throws Exception {
    JSONObject response = new JSONObject(NativeBridge.request(data.toString()));
    if (!response.optBoolean("ok")) throw new Exception(response.optString("error"));
    return response.getJSONArray("data");
  }

  void command(JSONObject command) throws JSONException {
    switch (command.optString("op")) {
      case "pause":
        pausedByUser = player.getPlayWhenReady();
        if (pausedByUser) player.pause();
        else {
          if (player.getPlaybackState() == Player.STATE_ENDED) player.seekTo(0);
          player.play();
        }
        break;
      case "stop":
        stopAndClear();
        break;
      case "forward":
        player.seekTo(player.getCurrentPosition() + 10000);
        break;
      case "back":
        player.seekTo(Math.max(0, player.getCurrentPosition() - 10000));
        break;
      case "seek":
        player.seekTo((long) (command.optDouble("seconds") * 1000));
        break;
      case "volume":
        float volume = (float) Math.max(0, Math.min(1, command.optDouble("value", 0.7)));
        player.setVolume(volume);
        prefs.edit().putFloat("volume", volume).apply();
        break;
      case "speed":
        float speed = (float) Math.max(0.25, Math.min(4, command.optDouble("value", 1)));
        player.setPlaybackSpeed(speed);
        prefs.edit().putFloat("speed", speed).apply();
        break;
      case "next":
        next();
        break;
      case "previous":
        player.seekTo(0);
        break;
      case "repeat":
        player.setRepeatMode(
            player.getRepeatMode() == Player.REPEAT_MODE_OFF
                ? Player.REPEAT_MODE_ONE
                : Player.REPEAT_MODE_OFF);
        break;
      case "shuffle":
        player.setShuffleModeEnabled(!player.getShuffleModeEnabled());
        break;
      case "sponsor_replay":
        if (lastSkipped >= 0) {
          skipUntil = player.getCurrentPosition() + 1000;
          player.seekTo(lastSkipped);
        }
        break;
      case "settings":
        JSONObject config = command.optJSONObject("config");
        if (config != null) {
          skipSponsors = "skip".equals(config.optString("playback_sponsorblock"));
          historyEnabled = config.optBoolean("playback_history", true);
          AudioTap.enabled =
              config.optString("music_player_style").startsWith("vu")
                  || config.optString("music_player_style").startsWith("orbit");
        }
        break;
      case "track":
        int group = command.optInt("group", -1), index = command.optInt("index", -1);
        java.util.List<Tracks.Group> groups = player.getCurrentTracks().getGroups();
        if (group >= 0 && group < groups.size() && index >= 0 && index < groups.get(group).length) {
          Tracks.Group selected = groups.get(group);
          player.setTrackSelectionParameters(
              player
                  .getTrackSelectionParameters()
                  .buildUpon()
                  .setTrackTypeDisabled(selected.getType(), false)
                  .setOverrideForType(
                      new TrackSelectionOverride(selected.getMediaTrackGroup(), index))
                  .build());
        }
        break;
      case "captions_toggle":
        boolean disabled =
            player.getTrackSelectionParameters().disabledTrackTypes.contains(C.TRACK_TYPE_TEXT);
        player.setTrackSelectionParameters(
            player
                .getTrackSelectionParameters()
                .buildUpon()
                .setTrackTypeDisabled(C.TRACK_TYPE_TEXT, !disabled)
                .build());
        break;
      case "sleep":
        sleepDeadline = SystemClock.elapsedRealtime() + command.optLong("seconds", 1800) * 1000;
        break;
    }
    publish();
  }

  boolean isLive() {
    JSONObject media = currentRequest.optJSONObject("media");
    return media != null && media.optBoolean("live") && !currentRequest.optBoolean("preview");
  }

  static boolean rateLimited(Throwable error) {
    for (Throwable cause = error; cause != null; cause = cause.getCause()) {
      if (cause instanceof DefaultHttpDataSource.InvalidResponseCodeException
          && ((DefaultHttpDataSource.InvalidResponseCodeException) cause).responseCode == 429)
        return true;
    }
    return false;
  }

  void reconnect() {
    int attempt = ++reconnectCount;
    long version = ++generation;
    error = "Live stream disconnected; reconnecting (" + attempt + "/3)";
    publish();
    handler.postDelayed(
        () -> {
          if (version != generation || current != this) return;
          worker.execute(
              () -> {
                try {
                  JSONObject media = currentRequest.getJSONObject("media");
                  JSONObject stream =
                      rpc(
                          new JSONObject()
                              .put("op", "resolve")
                              .put("media", media)
                              .put("audio", currentRequest.optBoolean("audio")));
                  JSONObject request =
                      new JSONObject(currentRequest.toString())
                          .put("stream", stream)
                          .put("reconnect", true);
                  handler.post(
                      () -> {
                        if (version == generation && current == this)
                          try {
                            play(request);
                          } catch (Exception e) {
                            error = "Reconnect could not start";
                            publish();
                          }
                      });
                } catch (Exception e) {
                  handler.post(
                      () -> {
                        if (version == generation && current == this) {
                          if (reconnectCount < 3) reconnect();
                          else {
                            error = "Live stream reconnect limit reached";
                            publish();
                          }
                        }
                      });
                }
              });
        },
        (1L << attempt) * 1000);
  }

  void next() {
    boolean shuffle = player.getShuffleModeEnabled();
    long version = ++generation;
    worker.execute(
        () -> {
          try {
            JSONObject media =
                rpc(new JSONObject().put("op", "queue_peek").put("shuffle", shuffle));
            if (media.length() == 0) return;
            boolean audio =
                "Song".equals(media.optString("kind")) || "Station".equals(media.optString("kind"));
            JSONObject stream =
                rpc(new JSONObject().put("op", "resolve").put("media", media).put("audio", audio));
            JSONObject request =
                new JSONObject().put("media", media).put("stream", stream).put("audio", audio);
            handler.post(
                () -> {
                  if (version == generation)
                    try {
                      play(request);
                      worker.execute(
                          () -> {
                            try {
                              rpc(
                                  new JSONObject()
                                      .put("op", "queue_commit")
                                      .put("url", media.optString("url")));
                            } catch (Exception ignored) {
                            }
                          });
                    } catch (Exception ignored) {
                    }
                });
          } catch (Exception e) {
            handler.post(
                () -> {
                  error = "Could not play the next queued item";
                  publish();
                });
          }
        });
  }

  void stopAndClear() {
    generation++;
    player.stop();
    player.clearMediaItems();
    currentRequest = new JSONObject();
    publish();
    stopSelf();
  }

  final Runnable ticker =
      new Runnable() {
        @Override
        public void run() {
          if (current != PlaybackService.this) return;
          long pos = player.getCurrentPosition();
          if (player.isPlaying() && skipSponsors && pos >= skipUntil) {
            for (int i = 0; i < sponsors.length(); i++) {
              JSONObject segment = sponsors.optJSONObject(i);
              if (segment == null) continue;
              long start = (long) (segment.optDouble("start") * 1000),
                  end = (long) (segment.optDouble("end") * 1000);
              if (pos >= start && pos < end) {
                lastSkipped = start;
                skipUntil = end;
                player.seekTo(end);
                break;
              }
            }
          }
          if (sleepDeadline > 0 && SystemClock.elapsedRealtime() >= sleepDeadline) {
            player.pause();
            pausedByUser = true;
            sleepDeadline = 0;
          }
          if (historyEnabled
              && player.isPlaying()
              && !currentRequest.optBoolean("preview")
              && SystemClock.elapsedRealtime() - lastPositionSave > 5000) {
            lastPositionSave = SystemClock.elapsedRealtime();
            JSONObject media = currentRequest.optJSONObject("media");
            if (media != null) {
              String url = media.optString("url");
              worker.execute(
                  () -> {
                    try {
                      rpc(
                          new JSONObject()
                              .put("op", "position")
                              .put("url", url)
                              .put("milliseconds", pos));
                    } catch (Exception ignored) {
                    }
                  });
            }
          }
          publish();
          handler.postDelayed(this, 200);
        }
      };

  void publish() {
    try {
      JSONObject media = currentRequest.optJSONObject("media");
      JSONObject status =
          new JSONObject()
              .put("title", media == null ? "" : media.optString("title"))
              .put("playing", player.isPlaying())
              .put("paused", pausedByUser)
              .put("buffering", player.getPlaybackState() == Player.STATE_BUFFERING)
              .put("position", player.getCurrentPosition() / 1000.0)
              .put("duration", Math.max(0, player.getDuration()) / 1000.0)
              .put("video", !currentRequest.optBoolean("audio", true))
              .put("captions", captionText)
              .put("segments", sponsors)
              .put("volume", player.getVolume())
              .put("speed", player.getPlaybackParameters().speed)
              .put("error", error.isEmpty() ? JSONObject.NULL : error);
      if (AudioTap.enabled) {
        long packed = AudioTap.levels;
        status
            .put("left", (packed & 65535) / 65535.0)
            .put("right", ((packed >>> 16) & 65535) / 65535.0)
            .put("peak", ((packed >>> 32) & 65535) / 65535.0);
      }
      status.put("media", media);
      JSONArray tracks = new JSONArray();
      java.util.List<Tracks.Group> groups = player.getCurrentTracks().getGroups();
      for (int g = 0; g < groups.size(); g++) {
        Tracks.Group group = groups.get(g);
        for (int i = 0; i < group.length; i++) {
          Format format = group.getTrackFormat(i);
          tracks.put(
              new JSONObject()
                  .put("group", g)
                  .put("index", i)
                  .put(
                      "label",
                      (group.getType() == C.TRACK_TYPE_AUDIO
                              ? "Audio "
                              : group.getType() == C.TRACK_TYPE_TEXT ? "Caption " : "Video ")
                          + (format.language == null ? "" : format.language)
                          + " "
                          + (format.label == null ? "" : format.label)
                          + " "
                          + (format.height > 0 ? format.height + "p" : "")));
        }
      }
      status.put("tracks", tracks);
      MediaBridge.playerSnapshot = status.toString();
    } catch (JSONException ignored) {
    }
  }

  @Override
  public void onDestroy() {
    current = null;
    handler.removeCallbacksAndMessages(null);
    worker.shutdownNow();
    session.release();
    player.release();
    MediaBridge.playerSnapshot = "{}";
    super.onDestroy();
  }
}
