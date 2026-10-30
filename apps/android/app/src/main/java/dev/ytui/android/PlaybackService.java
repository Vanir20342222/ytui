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
  private static boolean emulatorFixture() {
    return BuildConfig.DEBUG
        && MediaBridge.offlineUi
        && (android.os.Build.HARDWARE.equals("ranchu")
            || android.os.Build.HARDWARE.equals("goldfish"));
  }

  static volatile PlaybackService current;
  ExoPlayer player;
  Player controlsPlayer;
  MediaSession session;
  LiveMetadataPlayer sessionPlayer;
  final ExecutorService worker = Executors.newFixedThreadPool(3);
  final Handler handler = new Handler(Looper.getMainLooper());
  final Set<Long> contextJobs = ConcurrentHashMap.newKeySet();
  volatile long generation = 0;
  long qualityIntent = 0;
  long skipUntil = 0, lastSkipped = -1, lastPositionSave = 0, sleepDeadline = 0;
  boolean pendingStart, navigationPending, contextReady, contextPreparing, waitingForContext;
  int queuedNext;
  String activeContext = "", playbackQuality = "1080", relatedForUrl = "";
  long startupAt, firstFrameMs, startMs, startupToken;

  void visibleFrame() {
    StartupTimings.output(startupToken, SystemClock.elapsedRealtime(), true);
    publish();
  }

  long streamMetadataUpdates;
  String sponsorMode = "skip";
  String error = "", streamTitle = "", captionText = "", pendingQueueUrl = "", errorTrace = "";
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
    android.app.NotificationManager notifications =
        getSystemService(android.app.NotificationManager.class);
    notifications.createNotificationChannel(
        new android.app.NotificationChannel(
            "ytui-playback", "ytui playback", android.app.NotificationManager.IMPORTANCE_LOW));
    setMediaNotificationProvider(
        new DefaultMediaNotificationProvider.Builder(this)
            .setNotificationId(NotificationIds.PLAYBACK)
            .setChannelId("ytui-playback")
            .build());
    prefs = getSharedPreferences("player", MODE_PRIVATE);
    player =
        new ExoPlayer.Builder(
                this,
                new DefaultRenderersFactory(this) {
                  @Override
                  protected void buildVideoRenderers(
                      android.content.Context ctx,
                      int extensionMode,
                      androidx.media3.exoplayer.mediacodec.MediaCodecSelector selector,
                      boolean fallback,
                      Handler events,
                      androidx.media3.exoplayer.video.VideoRendererEventListener listener,
                      long joiningMs,
                      java.util.ArrayList<Renderer> renderers) {
                    if (!emulatorFixture()) {
                      super.buildVideoRenderers(
                          ctx,
                          extensionMode,
                          selector,
                          fallback,
                          events,
                          listener,
                          joiningMs,
                          renderers);
                      return;
                    }
                    // Goldfish can migrate zero host color-buffer IDs from a
                    // placeholder to a TextureView even with the C2 software
                    // codec. Its offline fixture must initialize against the
                    // actual surface and recreate on surface changes. Physical
                    // devices/release builds keep Media3's standard renderer.
                    renderers.add(
                        new androidx.media3.exoplayer.video.MediaCodecVideoRenderer(
                            ctx, selector, joiningMs, fallback, events, listener, 50) {
                          @Override
                          protected boolean shouldUsePlaceholderSurface(
                              androidx.media3.exoplayer.mediacodec.MediaCodecInfo codec) {
                            return false;
                          }

                          @Override
                          protected boolean shouldUseDetachedSurface(
                              androidx.media3.exoplayer.mediacodec.MediaCodecInfo codec) {
                            return false;
                          }

                          @Override
                          protected boolean codecNeedsSetOutputSurfaceWorkaround(String name) {
                            return true;
                          }
                        });
                  }

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
                }.setEnableDecoderFallback(true)
                    .setMediaCodecSelector(
                        (mime, secure, tunneling) -> {
                          java.util.List<androidx.media3.exoplayer.mediacodec.MediaCodecInfo>
                              decoders =
                                  androidx.media3.exoplayer.mediacodec.MediaCodecSelector.DEFAULT
                                      .getDecoderInfos(mime, secure, tunneling);
                          // Goldfish's host-FFmpeg AVC bridge can report decoded frames
                          // backed by invalid color-buffer IDs under software GPUs.
                          // Offline fixtures use Android's real software codec instead;
                          // normal apps/devices keep the default hardware selection.
                          if (emulatorFixture() && mime.startsWith("video/")) {
                            java.util.List<androidx.media3.exoplayer.mediacodec.MediaCodecInfo>
                                software = new java.util.ArrayList<>();
                            for (androidx.media3.exoplayer.mediacodec.MediaCodecInfo decoder :
                                decoders)
                              if (decoder.name.startsWith("c2.android.")
                                  || decoder.name.startsWith("OMX.google.")) software.add(decoder);
                            if (!software.isEmpty()) return software;
                          }
                          return decoders;
                        }))
            .setLoadControl(
                new DefaultLoadControl.Builder()
                    .setBufferDurationsMs(5000, 20000, 250, 750)
                    .build())
            .build();
    player.setAudioAttributes(
        new AudioAttributes.Builder()
            .setUsage(C.USAGE_MEDIA)
            .setContentType(C.AUDIO_CONTENT_TYPE_MUSIC)
            .build(),
        true);
    player.setHandleAudioBecomingNoisy(true);
    player.setWakeMode(C.WAKE_MODE_NETWORK);
    player.setVideoFrameMetadataListener(
        (presentationTimeUs, releaseTimeNs, format, mediaFormat) ->
            MediaBridge.videoFrames.incrementAndGet());
    player.setVolume(prefs.getFloat("volume", 0.7f));
    player.setPlaybackSpeed(prefs.getFloat("speed", 1f));
    controlsPlayer = new ControlsPlayer(player);
    session =
        new MediaSession.Builder(
                this,
                sessionPlayer =
                    new LiveMetadataPlayer(player) {
                      @Override
                      protected com.google.common.util.concurrent.ListenableFuture<?>
                          handleSetPlayWhenReady(boolean ready) {
                        pausedByUser = !ready;
                        if (ready && player.getPlaybackState() == Player.STATE_ENDED)
                          player.seekTo(0);
                        return super.handleSetPlayWhenReady(ready);
                      }

                      @Override
                      protected com.google.common.util.concurrent.ListenableFuture<?> handleStop() {
                        stopAndClear();
                        return com.google.common.util.concurrent.Futures.immediateVoidFuture();
                      }

                      @Override
                      protected State getState() {
                        State state = super.getState();
                        return state
                            .buildUpon()
                            .setAvailableCommands(
                                state
                                    .availableCommands
                                    .buildUpon()
                                    .add(Player.COMMAND_SEEK_TO_NEXT)
                                    .add(Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM)
                                    .add(Player.COMMAND_SEEK_TO_PREVIOUS)
                                    .add(Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM)
                                    .build())
                            .build();
                      }

                      @Override
                      protected com.google.common.util.concurrent.ListenableFuture<?> handleSeek(
                          int index, long position, int command) {
                        switch (command) {
                          case Player.COMMAND_SEEK_TO_NEXT:
                          case Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM:
                            PlaybackService.this.next();
                            break;
                          case Player.COMMAND_SEEK_TO_PREVIOUS:
                            PlaybackService.this.previous();
                            break;
                          case Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM:
                            PlaybackService.this.navigate(true);
                            break;
                          default:
                            return super.handleSeek(index, position, command);
                        }
                        return com.google.common.util.concurrent.Futures.immediateVoidFuture();
                      }
                    })
            .setSessionActivity(
                PendingIntent.getActivity(
                    this,
                    0,
                    new Intent(this, RustActivity.class).setAction("dev.ytui.OPEN_PLAYER"),
                    PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT))
            .build();
    player.addListener(
        new Player.Listener() {
          @Override
          public void onMediaMetadataChanged(MediaMetadata metadata) {
            if (!isLive()) return;
            JSONObject media = currentRequest.optJSONObject("media");
            String title = PlaybackPolicy.liveTitle(metadata.title);
            if (!title.isEmpty() && media != null && !title.equals(media.optString("title").trim()))
              setLiveTitle(title);
          }

          @Override
          public void onMetadata(Metadata metadata) {
            for (int i = 0; i < metadata.length(); i++) {
              if (metadata.get(i) instanceof androidx.media3.extractor.metadata.icy.IcyInfo icy
                  && icy.title != null) setLiveTitle(icy.title);
            }
          }

          @Override
          public void onVideoSizeChanged(VideoSize size) {
            if (size.width < 1 || size.height < 1) return;
            MediaBridge.videoWidth = Math.max(1, size.width);
            MediaBridge.videoHeight = Math.max(1, size.height);
            synchronized (MediaBridge.class) {
              if (MediaBridge.texture != null)
                MediaBridge.texture.setDefaultBufferSize(
                    MediaBridge.videoWidth, MediaBridge.videoHeight);
            }
            publish();
          }

          @Override
          public void onPositionDiscontinuity(
              Player.PositionInfo oldPosition, Player.PositionInfo newPosition, int reason) {
            publish();
          }

          @Override
          public void onRenderedFirstFrame() {
            if (firstFrameMs == 0) {
              firstFrameMs = Math.max(1, SystemClock.elapsedRealtime() - startupAt);
              StartupTimings.stage(startupToken, "decoder", firstFrameMs);
            }
            publish();
          }

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
            pendingStart = navigationPending = waitingForContext = false;
            queuedNext = 0;
            StringBuilder trace = new StringBuilder();
            Throwable cause = e;
            for (int i = 0; cause != null && i < 4; i++, cause = cause.getCause()) {
              trace.append(cause.getClass().getName()).append("\n");
              StackTraceElement[] stack = cause.getStackTrace();
              for (int j = 0; j < Math.min(stack.length, 8); j++)
                trace.append(stack[j]).append("\n");
            }
            errorTrace = trace.toString();
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
            if (state == Player.STATE_READY)
              StartupTimings.stage(
                  startupToken, "ready", SystemClock.elapsedRealtime() - startupAt);
            if (state == Player.STATE_ENDED) {
              if (currentRequest.optBoolean("preview")) {
                finishPreview();
              } else if (isLive()) {
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
            if (playing
                && player.isPlaying()
                && player.getPlaybackState() == Player.STATE_READY
                && pendingStart) {
              recordStarted();
            }
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

  private void setLiveTitle(CharSequence raw) {
    if (!isLive()) return;
    String title = PlaybackPolicy.liveTitle(raw);
    if (title.equals(streamTitle)) return;
    streamTitle = title;
    streamMetadataUpdates++;
    sessionPlayer.metadataChanged();
    publish();
  }

  /**
   * Keep per-surface frame events synchronous for PlayerView's shutter/texture lifecycle. The
   * session's snapshot adapter remains responsible for external metadata/controllers.
   */
  private final class ControlsPlayer extends ForwardingPlayer {
    ControlsPlayer(Player delegate) {
      super(delegate);
    }

    @Override
    public Player.Commands getAvailableCommands() {
      return super.getAvailableCommands()
          .buildUpon()
          .add(Player.COMMAND_SEEK_TO_NEXT)
          .add(Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM)
          .add(Player.COMMAND_SEEK_TO_PREVIOUS)
          .add(Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM)
          .build();
    }

    @Override
    public boolean isCommandAvailable(int command) {
      return getAvailableCommands().contains(command);
    }

    @Override
    public void play() {
      setPlayWhenReady(true);
    }

    @Override
    public void pause() {
      setPlayWhenReady(false);
    }

    @Override
    public void setPlayWhenReady(boolean ready) {
      pausedByUser = !ready;
      if (ready && player.getPlaybackState() == Player.STATE_ENDED) player.seekTo(0);
      super.setPlayWhenReady(ready);
    }

    @Override
    public void stop() {
      stopAndClear();
    }

    @Override
    public void seekToNext() {
      PlaybackService.this.next();
    }

    @Override
    public void seekToNextMediaItem() {
      PlaybackService.this.next();
    }

    @Override
    public void seekToPrevious() {
      PlaybackService.this.previous();
    }

    @Override
    public void seekToPreviousMediaItem() {
      navigate(true);
    }
  }

  // MediaItem's fixed station title takes precedence in ExoPlayer metadata.
  // Overlay timed ICY information for session consumers without replacing the
  // source, restarting audio, or changing the station's durable media identity.
  private class LiveMetadataPlayer extends ForwardingSimpleBasePlayer {
    LiveMetadataPlayer(Player delegate) {
      super(delegate);
    }

    void metadataChanged() {
      invalidateState();
    }

    @Override
    protected State getState() {
      State state = super.getState();
      if (isLive() && !streamTitle.isEmpty() && !state.timeline.isEmpty()) {
        return state
            .buildUpon()
            .setPlaylist(
                state.timeline,
                state.currentTracks,
                state.currentMetadata.buildUpon().setTitle(streamTitle).build())
            .build();
      }
      return state;
    }
  }

  @Override
  public MediaSession onGetSession(MediaSession.ControllerInfo controller) {
    return session;
  }

  @Override
  public int onStartCommand(Intent intent, int flags, int id) {
    if (intent == null) {
      stopSelf(id);
      return START_NOT_STICKY;
    }
    boolean startsPlayback = "play".equals(intent.getAction());
    if (startsPlayback) {
      try {
        // The external session must leave EMPTY before notification updates run.
        // Otherwise System UI can remove the preparation notification and send
        // a delayed Stop to the newly playing session.
        play(new JSONObject(intent.getStringExtra("request")));
        sessionPlayer.metadataChanged();
        // Direct app playback has no controller bind to register the session.
        // Register only now, so its first notification update sees BUFFERING
        // rather than the empty onCreate state.
        if (!getSessions().contains(session)) addSession(session);
      } catch (Exception e) {
        error = "Invalid playback request";
        publish();
        stopForeground(STOP_FOREGROUND_REMOVE);
        stopSelf();
        return START_NOT_STICKY;
      }
    }
    // Media3 owns the notification and foreground lifecycle. A hand-built
    // notification can be posted before its asynchronous controller connects;
    // an early default refresh then removes it and System UI sends Stop to
    // the new session. Let the connected controller publish the first one.
    return super.onStartCommand(intent, flags, id);
  }

  void play(JSONObject request) throws JSONException {
    // Resolution is finished: hand cached bytes to the foreground reader and
    // stop speculative prefix I/O competing with initial audio/video buffering.
    // Cancellation retains completed cache spans and Rust stream leases.
    PlaybackCache.cancel();
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
    String nextContext = request.isNull("context_id") ? "" : request.optString("context_id");
    boolean sameContext = nextContext.equals(activeContext);
    if (!sameContext && !request.optBoolean("preview")) {
      cancelContextJobs();
      activeContext = nextContext;
      contextReady = contextPreparing = waitingForContext = false;
      queuedNext = 0;
    }
    // A preview must not invalidate the interrupted collection.
    if (!request.optBoolean("preview")) NativeBridge.activateContext(nextContext);
    generation++;
    startupAt = SystemClock.elapsedRealtime();
    firstFrameMs = startMs = 0;
    currentRequest = request;
    startupToken = request.optLong("startup_token");
    StartupTimings.handoff(startupToken, startupAt);
    pendingStart = !request.optBoolean("preview");
    navigationPending = false;
    playbackQuality = request.optString("quality", playbackQuality);
    error = "";
    errorTrace = "";
    streamTitle = "";
    streamMetadataUpdates = 0;
    captionText = "";
    skipUntil = 0;
    lastSkipped = -1;
    sponsors = new JSONArray();
    JSONObject stream = request.getJSONObject("stream"), media = request.getJSONObject("media");
    if (stream.optBoolean("live")) media.put("live", true);
    pendingQueueUrl = request.optBoolean("queued") ? media.optString("url") : "";
    DefaultHttpDataSource.Factory http =
        new DefaultHttpDataSource.Factory()
            .setConnectTimeoutMs(8000)
            .setReadTimeoutMs(12000)
            .setAllowCrossProtocolRedirects(false)
            .setDefaultRequestProperties(headers(stream.optJSONObject("headers")));
    DataSource.Factory upstream =
        request.optBoolean("offline")
            ? OfflineStore.offlineFactory()
            : request.optBoolean("local") || media.optBoolean("live") || stream.optBoolean("live")
                ? new androidx.media3.datasource.DefaultDataSource.Factory(this, http)
                : PlaybackCache.source(stream);
    long networkToken = startupToken, networkStart = startupAt;
    java.util.concurrent.atomic.AtomicBoolean responseSeen =
        new java.util.concurrent.atomic.AtomicBoolean();
    java.util.concurrent.atomic.AtomicBoolean byteSeen =
        new java.util.concurrent.atomic.AtomicBoolean();
    DataSource.Factory traced =
        () -> {
          DataSource dataSource = upstream.createDataSource();
          dataSource.addTransferListener(
              new TransferListener() {
                @Override
                public void onTransferInitializing(
                    DataSource source, DataSpec spec, boolean network) {}

                @Override
                public void onTransferStart(DataSource source, DataSpec spec, boolean network) {
                  if (network && responseSeen.compareAndSet(false, true))
                    StartupTimings.stage(
                        networkToken, "net_response", SystemClock.elapsedRealtime() - networkStart);
                }

                @Override
                public void onBytesTransferred(
                    DataSource source, DataSpec spec, boolean network, int bytes) {
                  if (network && bytes > 0 && byteSeen.compareAndSet(false, true))
                    StartupTimings.stage(
                        networkToken, "net_byte", SystemClock.elapsedRealtime() - networkStart);
                }

                @Override
                public void onTransferEnd(DataSource source, DataSpec spec, boolean network) {}
              });
          return dataSource;
        };
    DefaultMediaSourceFactory sources =
        new DefaultMediaSourceFactory(traced)
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
    if (choices != null && !request.optBoolean("offline")) {
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
    String hlsMaster = request.optBoolean("offline") ? "" : stream.optString("hls_master");
    // Rust preserves the chosen muxed HLS variant's CODECS in a small in-memory
    // master. Media3 can prepare tracks without first sniffing a media segment.
    // Downloads/offline reads retain their original URIs and cache identities.
    boolean chunkless = !hlsMaster.isEmpty() && hlsMaster.length() <= 65536;
    int contentType =
        androidx.media3.common.util.Util.inferContentTypeForUriAndMimeType(
            android.net.Uri.parse(stream.getString("url")), null);
    StartupTimings.presentation(
        startupToken,
        playbackQuality,
        request.optBoolean("offline")
            ? "offline"
            : chunkless
                ? "hls-chunkless"
                : contentType == C.CONTENT_TYPE_HLS
                    ? "hls"
                    : contentType == C.CONTENT_TYPE_DASH ? "dash" : "progressive");
    String sourceUri =
        chunkless
            ? "data:application/vnd.apple.mpegurl;base64,"
                + android.util.Base64.encodeToString(
                    hlsMaster.getBytes(java.nio.charset.StandardCharsets.UTF_8),
                    android.util.Base64.NO_WRAP)
            : stream.getString("url");
    MediaItem main =
        new MediaItem.Builder()
            .setUri(sourceUri)
            .setMimeType(chunkless ? MimeTypes.APPLICATION_M3U8 : null)
            .setCustomCacheKey(
                chunkless || stream.isNull("cache_key") ? null : stream.optString("cache_key"))
            .setMediaId(media.optString("url"))
            .setSubtitleConfigurations(captions)
            .setMediaMetadata(metadata)
            .build();
    MediaSource source = sources.createMediaSource(main);
    String audio = stream.optString("audio_url", "");
    if (!audio.isEmpty() && !audio.equals("null"))
      source =
          new MergingMediaSource(
              true,
              source,
              sources.createMediaSource(
                  new MediaItem.Builder()
                      .setUri(audio)
                      .setCustomCacheKey(
                          stream.isNull("audio_cache_key")
                              ? null
                              : stream.optString("audio_cache_key"))
                      .build()));
    player.setMediaSource(source);
    player.prepare();
    if (pausedByUser) player.pause();
    else player.play();
    if (request.optBoolean("preview")) {
      long version = generation;
      player.setVolume(0f);
      handler.postDelayed(
          () -> {
            if (version == generation) finishPreview();
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
            if (request.optBoolean("offline") || request.optBoolean("local")) return;
            JSONArray marks = rpcArray(new JSONObject().put("op", "sponsor").put("media", media));
            handler.post(
                () -> {
                  if (version == generation && !"off".equals(sponsorMode)) sponsors = marks;
                });
          } catch (Exception ignored) {
          }
        });
    publish();
  }

  void recordStarted() {
    if (!pendingStart || !player.isPlaying() || currentRequest.optBoolean("preview")) return;
    pendingStart = false;
    startMs = SystemClock.elapsedRealtime() - startupAt;
    if (currentRequest.optBoolean("audio"))
      StartupTimings.output(startupToken, SystemClock.elapsedRealtime(), false);
    try {
      JSONObject started = new JSONObject(currentRequest.toString());
      String contextId = contextId();
      NativeBridge.activateContext(contextId);
      String queuedUrl = pendingQueueUrl;
      pendingQueueUrl = "";
      currentRequest.remove("queued");
      MediaBridge.playbackStore.execute(
          () -> {
            try {
              if (!queuedUrl.isEmpty())
                rpc(
                    new JSONObject()
                        .put("op", "queue_commit")
                        .put("url", queuedUrl)
                        .put("context_id", contextId)
                        .put("context_entry", started.optBoolean("context_entry")));
              rpc(
                  new JSONObject()
                      .put("op", "history_record")
                      .put("media", started.getJSONObject("media"))
                      .put("index", started.optInt("history_index", -1)));
              MediaBridge.event("history_updated", "");
            } catch (Exception error) {
              MediaBridge.event("error", "Could not save the playback queue/history");
            }
            handler.post(
                () -> {
                  if (current != this || !contextId.equals(contextId())) return;
                  prepareContext(started.optJSONObject("context"));
                  refreshPrefetch();
                  if (queuedNext > 0) {
                    queuedNext--;
                    navigate(false);
                  }
                });
          });
    } catch (JSONException error) {
      MediaBridge.event("error", "Could not record the started media");
    }
  }

  String contextId() {
    return currentRequest.isNull("context_id") ? "" : currentRequest.optString("context_id");
  }

  void prepareContext(JSONObject descriptor) {
    if (descriptor == null || contextPreparing || contextReady || contextId().isEmpty()) return;
    String contextId = contextId();
    if (!contextId.equals(descriptor.optString("context_id"))) return;
    contextPreparing = true;
    worker.execute(
        () -> {
          long requestId = java.util.UUID.randomUUID().getMostSignificantBits() & Long.MAX_VALUE;
          contextJobs.add(requestId);
          try {
            JSONObject collection = new JSONObject(descriptor.toString());
            if ("offline_context".equals(collection.optString("op"))) {
              collection.put("request_id", requestId);
              JSONObject response = rpc(collection);
              if (response.optBoolean("prepared"))
                handler.post(() -> contextAvailable(contextId, true));
              return;
            }
            JSONArray seed = collection.optJSONArray("items");
            collection.remove("items");
            if (seed != null) {
              JSONObject response =
                  rpc(
                      new JSONObject()
                          .put("op", "seed_context")
                          .put("context_id", contextId)
                          .put("source_url", collection.optString("source_url"))
                          .put("items", seed));
              if (response.optInt("count") > 0)
                handler.post(() -> contextAvailable(contextId, false));
            }
            collection.put("request_id", requestId);
            JSONObject response = rpc(collection);
            if (response.optBoolean("prepared"))
              handler.post(() -> contextAvailable(contextId, true));
          } catch (Exception error) {
            if (contextJobs.contains(requestId))
              handler.post(
                  () -> {
                    if (current != this || !contextId.equals(contextId())) return;
                    contextPreparing = false;
                    if (waitingForContext) {
                      waitingForContext = false;
                      navigationPending = false;
                      queuedNext = 0;
                      this.error =
                          "Could not load the next items. Retry Next or choose another track.";
                      publish();
                    }
                  });
          } finally {
            contextJobs.remove(requestId);
          }
        });
  }

  void contextAvailable(String id, boolean complete) {
    if (current != this || !id.equals(activeContext)) return;
    if (complete) {
      contextReady = true;
      contextPreparing = false;
    }
    if (!id.equals(contextId()) || currentRequest.optBoolean("preview")) return;
    refreshPrefetch();
    if (waitingForContext
        || (!navigationPending
            && player.getPlaybackState() == Player.STATE_ENDED
            && !pausedByUser)) {
      waitingForContext = false;
      navigationPending = false;
      navigate(false);
    }
  }

  void refreshPrefetch() {
    if (currentRequest.optJSONObject("media") == null
        || currentRequest.optBoolean("preview")
        || isLive()) {
      PlaybackCache.cancel();
      return;
    }
    String id = contextId(), quality = playbackQuality;
    boolean shuffle = player.getShuffleModeEnabled();
    long version = generation;
    afterPlaybackStored(
        () -> {
          try {
            JSONObject candidate = candidate(false, shuffle, id);
            handler.post(
                () -> {
                  if (current != this || version != generation) return;
                  JSONObject media = candidate.optJSONObject("media");
                  if (media == null) PlaybackCache.cancel();
                  else
                    PlaybackCache.prefetch(
                        media, PlaybackPolicy.audioOnly(media.optString("kind")), quality);
                });
          } catch (Exception ignored) {
            /* warming is optional */
          }
        });
  }

  JSONObject candidate(boolean previous, boolean shuffle, String id) throws Exception {
    JSONObject media =
        previous
            ? new JSONObject()
            : rpc(
                new JSONObject()
                    .put("op", "queue_peek")
                    .put("shuffle", shuffle)
                    .put("preferred_url", PlaybackCache.upcomingUrl));
    boolean queued = media.length() > 0;
    int historyIndex = -1;
    if (media.length() == 0) {
      JSONObject history =
          rpc(
              new JSONObject()
                  .put("op", "history_peek")
                  .put("direction", previous ? "previous" : "next"));
      if (history.has("media")) {
        media = history.getJSONObject("media");
        historyIndex = history.getInt("index");
      } else if (!previous) {
        media =
            rpc(
                new JSONObject()
                    .put("op", "queue_peek")
                    .put("shuffle", shuffle)
                    .put("context_id", id)
                    .put("preferred_url", PlaybackCache.upcomingUrl));
        queued = media.length() > 0;
      }
    }
    JSONObject value = new JSONObject().put("queued", queued).put("history_index", historyIndex);
    if (media.length() > 0) value.put("media", media);
    return value;
  }

  void finishPreview() {
    JSONObject restore = previewRestore;
    long position = previewPosition;
    boolean wasPlaying = previewPlaying;
    previewRestore = null;
    try {
      if (restore != null) {
        play(restore);
        player.seekTo(position);
        pausedByUser = !wasPlaying;
        if (!wasPlaying) player.pause();
      } else {
        generation++;
        currentRequest.remove("preview");
        pausedByUser = true;
        player.pause();
        player.setVolume(prefs.getFloat("volume", 0.7f));
      }
    } catch (Exception error) {
      player.pause();
      this.error = "Could not restore playback after preview";
    }
    publish();
  }

  static void mergeMetadata(JSONObject media, JSONObject stream) throws JSONException {
    for (String key :
        new String[] {"title", "creator", "thumbnail", "upload_date", "description"}) {
      String value = stream.optString(key);
      if (!value.isEmpty() && !value.equals("null")) media.put(key, value);
    }
    for (String key :
        new String[] {"duration", "view_count", "like_count", "channel_follower_count"}) {
      double value = stream.optDouble(key, -1);
      if (Double.isFinite(value) && value >= 0 && value <= Long.MAX_VALUE)
        media.put(key, (long) value);
    }
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
        player.seekTo(player.getCurrentPosition() + MobileSettings.integer("seek_seconds") * 1000L);
        break;
      case "back":
        player.seekTo(
            Math.max(
                0, player.getCurrentPosition() - MobileSettings.integer("seek_seconds") * 1000L));
        break;
      case "seek":
        player.seekTo((long) (command.optDouble("seconds") * 1000));
        break;
      case "preview_end":
        finishPreview();
        break;
      case "preview_play_full":
        generation++;
        previewRestore = null;
        currentRequest.remove("preview");
        currentRequest.put("context_id", command.opt("context_id"));
        currentRequest.put("context", command.opt("context"));
        pendingStart = true;
        pausedByUser = false;
        player.setVolume(prefs.getFloat("volume", 0.7f));
        player.seekTo(0);
        player.play();
        if (player.isPlaying()) recordStarted();
        break;
      case "volume":
        if (currentRequest.optBoolean("preview")) break;
        float volume = (float) Math.max(0, Math.min(1, command.optDouble("value", 0.7)));
        player.setVolume(volume);
        prefs.edit().putFloat("volume", volume).apply();
        break;
      case "speed":
        if (currentRequest.optBoolean("preview")) break;
        float speed = (float) Math.max(0.25, Math.min(4, command.optDouble("value", 1)));
        player.setPlaybackSpeed(speed);
        prefs.edit().putFloat("speed", speed).apply();
        break;
      case "quality":
        changeQuality(command.optString("value"));
        break;
      case "next":
        next();
        break;
      case "previous":
        previous();
        break;
      case "repeat":
        player.setRepeatMode(
            player.getRepeatMode() == Player.REPEAT_MODE_OFF
                ? Player.REPEAT_MODE_ONE
                : Player.REPEAT_MODE_OFF);
        break;
      case "shuffle":
        player.setShuffleModeEnabled(!player.getShuffleModeEnabled());
        PlaybackCache.cancel();
        refreshPrefetch();
        break;
      case "queue_changed":
        refreshPrefetch();
        break;
      case "sponsor_watch":
        skipUntil = (long) (command.optDouble("end") * 1000) + 1;
        player.seekTo((long) (command.optDouble("start") * 1000));
        break;
      case "sleep_cancel":
        sleepDeadline = 0;
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
          String previousMode = sponsorMode;
          sponsorMode = config.optString("playback_sponsorblock", "skip");
          skipSponsors = "skip".equals(sponsorMode);
          if ("off".equals(sponsorMode)) sponsors = new JSONArray();
          else if (!previousMode.equals(sponsorMode)) {
            JSONObject media = currentRequest.optJSONObject("media");
            long version = generation;
            if (media != null)
              worker.execute(
                  () -> {
                    try {
                      JSONArray marks =
                          rpcArray(new JSONObject().put("op", "sponsor").put("media", media));
                      handler.post(
                          () -> {
                            if (version == generation && !"off".equals(sponsorMode))
                              sponsors = marks;
                          });
                    } catch (Exception ignored) {
                    }
                  });
          }
          historyEnabled = config.optBoolean("playback_history", true);
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

  private void changeQuality(String quality) throws JSONException {
    if (!Arrays.asList("360", "480", "720", "1080", "best").contains(quality)) return;
    if (currentRequest.optJSONObject("media") == null
        || currentRequest.optBoolean("offline")
        || currentRequest.optBoolean("local")
        || currentRequest.optBoolean("preview")
        || isLive()) {
      MediaBridge.event("notice", "Quality selection is available for online videos");
      return;
    }
    long intent = ++qualityIntent, version = generation;
    JSONObject original = currentRequest;
    JSONObject resolve =
        new JSONObject()
            .put("op", "resolve")
            .put("media", original.getJSONObject("media"))
            .put("audio", original.optBoolean("audio"))
            .put("quality", quality);
    MediaBridge.event("notice", "Loading quality " + quality + "…");
    worker.execute(
        () -> {
          try {
            JSONObject stream = rpc(resolve);
            handler.post(
                () -> {
                  if (current != this
                      || version != generation
                      || intent != qualityIntent
                      || currentRequest != original) return;
                  try {
                    replaceQuality(original, stream, quality);
                  } catch (JSONException invalid) {
                    MediaBridge.event("error", "Could not change video quality");
                  }
                });
          } catch (Exception unavailable) {
            handler.post(
                () -> {
                  if (current == this && version == generation && intent == qualityIntent)
                    MediaBridge.event(
                        "error", "Could not load this quality. Current playback was kept.");
                });
          }
        });
  }

  // Invoked on the player thread only; preserve user intent and finite collection state.
  void replaceQuality(JSONObject original, JSONObject stream, String quality) throws JSONException {
    long position = player.getCurrentPosition();
    boolean started = pendingStart, preparedContext = contextReady;
    JSONObject replacement =
        new JSONObject(original.toString())
            .put("stream", stream)
            .put("quality", quality)
            .put("reconnect", true)
            .put("resume", false)
            .put("queued", false);
    play(replacement);
    pendingStart = started;
    contextReady = preparedContext;
    player.seekTo(position);
    MediaBridge.event("notice", "Video quality: " + quality);
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
                          if (PreparationPolicy.rateLimited(String.valueOf(e.getMessage()))) {
                            error = "Service rate limit; wait before reconnecting";
                            publish();
                          } else if (reconnectCount < 3) reconnect();
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
    if (navigationPending || pendingStart) {
      queuedNext = Math.min(queuedNext + 1, 20);
      return;
    }
    navigate(false);
  }

  void previous() {
    if (!isLive() && player.getCurrentPosition() > 3000) player.seekTo(0);
    else navigate(true);
  }

  void afterPlaybackStored(Runnable action) {
    MediaBridge.playbackStore.execute(
        () -> {
          if (current != this) return;
          try {
            worker.execute(action);
          } catch (RejectedExecutionException ignored) {
          }
        });
  }

  void navigate(boolean previous) {
    long timing =
        StartupTimings.begin(SystemClock.elapsedRealtime(), !currentRequest.optBoolean("audio"));
    boolean shuffle = player.getShuffleModeEnabled();
    String contextId = contextId(), quality = playbackQuality;
    long version = ++generation;
    navigationPending = true;
    publish();
    afterPlaybackStored(
        () -> {
          try {
            if (version != generation || current != this) return;
            JSONObject chosen = candidate(previous, shuffle, contextId);
            JSONObject media = chosen.optJSONObject("media");
            boolean queued = chosen.optBoolean("queued");
            int historyIndex = chosen.optInt("history_index", -1);
            if (media == null) {
              handler.post(
                  () -> {
                    if (version != generation || current != this) return;
                    if (!previous && contextPreparing) {
                      waitingForContext = true;
                      publish();
                      return;
                    }
                    if (!previous && prepareRelated()) {
                      waitingForContext = true;
                      publish();
                      return;
                    }
                    navigationPending = false;
                    queuedNext = 0;
                    if (previous && !isLive()) player.seekTo(0);
                    publish();
                  });
              return;
            }
            boolean audio = PlaybackPolicy.audioOnly(media.optString("kind"));
            StartupTimings.mediaType(timing, !audio);
            StartupTimings.stage(
                timing, "selection", StartupTimings.elapsed(timing, SystemClock.elapsedRealtime()));
            long offlineAt = SystemClock.elapsedRealtime();
            String savedId = media.optString("offline_id");
            JSONObject request =
                !savedId.isEmpty() && !savedId.equals("null")
                    ? OfflineStore.offlineRequest(savedId)
                    : MobileSettings.enabled("prefer_offline")
                        ? OfflineStore.find(media, audio, quality)
                        : null;
            StartupTimings.stage(timing, "offline", SystemClock.elapsedRealtime() - offlineAt);
            if (request == null) {
              JSONObject stream =
                  rpc(
                      new JSONObject()
                          .put("op", "resolve")
                          .put("media", media)
                          .put("audio", audio)
                          .put("quality", quality));
              mergeMetadata(media, stream);
              request =
                  new JSONObject().put("media", media).put("stream", stream).put("audio", audio);
            }
            StartupTimings.resolved(
                timing,
                SystemClock.elapsedRealtime(),
                request.optBoolean("offline")
                    || request.getJSONObject("stream").optBoolean("resolution_cache_hit"));
            if (!request.optBoolean("offline"))
              MediaBridge.recordResolutionTimings(timing, request.getJSONObject("stream"));
            StartupTimings.route(
                timing,
                request.optBoolean("offline")
                    ? "offline"
                    : request
                        .getJSONObject("stream")
                        .optString("resolution_route", "direct-media"));
            request
                .put("startup_token", timing)
                .put("quality", quality)
                .put("queued", queued)
                .put("history_index", historyIndex)
                .put("context_id", contextId)
                .put("context_entry", media.optBoolean("context_entry"));
            JSONObject ready = request;
            handler.post(
                () -> {
                  if (version != generation || current != this) return;
                  try {
                    play(ready);
                  } catch (Exception error) {
                    this.error = "Could not start the next queued item";
                    publish();
                  }
                });
          } catch (Exception error) {
            handler.post(
                () -> {
                  if (version == generation && current == this) {
                    navigationPending = false;
                    queuedNext = 0;
                    this.error = "Could not play the next queued item";
                    publish();
                  }
                });
          }
        });
  }

  boolean prepareRelated() {
    JSONObject media = currentRequest.optJSONObject("media");
    if (media == null
        || isLive()
        || currentRequest.optBoolean("offline")
        || currentRequest.optBoolean("local")
        || !media.isNull("offline_id")
        || currentRequest.optBoolean("preview")
        || MediaBridge.offlineUi
        || !MobileSettings.enabled("autoplay_related")) return false;
    String url = media.optString("url"), id = media.optString("id");
    if (url.equals(relatedForUrl)) return false;
    android.net.Uri uri = android.net.Uri.parse(url);
    String host = uri.getHost();
    if (!Arrays.asList("youtube.com", "www.youtube.com", "music.youtube.com", "youtu.be")
        .contains(host)) return false;
    if (!id.matches("[A-Za-z0-9_-]{11}")) {
      id = "youtu.be".equals(host) ? uri.getLastPathSegment() : uri.getQueryParameter("v");
      if (id == null || !id.matches("[A-Za-z0-9_-]{11}")) return false;
    }
    try {
      relatedForUrl = url;
      String token = UUID.randomUUID().toString();
      JSONObject descriptor =
          new JSONObject()
              .put("op", "prepare_context")
              .put("context_id", token)
              .put("source_url", url)
              .put("source_id", id)
              .put("music", currentRequest.optBoolean("audio"))
              .put("request", new JSONObject().put("Related", id));
      currentRequest.put("context_id", token).put("context", descriptor);
      activeContext = token;
      contextReady = false;
      NativeBridge.activateContext(token);
      prepareContext(descriptor);
      return true;
    } catch (JSONException impossible) {
      return false;
    }
  }

  void pauseForBackground() {
    if (!MobileSettings.enabled("background_playback") && player.getPlayWhenReady()) {
      pausedByUser = true;
      player.pause();
      publish();
    }
  }

  void stopAndClear() {
    generation++;
    NativeBridge.activateContext("");
    cancelContextJobs();
    PlaybackCache.cancel();
    activeContext = "";
    contextPreparing = contextReady = waitingForContext = false;
    queuedNext = 0;
    pendingStart = false;
    navigationPending = false;
    streamTitle = "";
    streamMetadataUpdates = 0;
    player.stop();
    player.clearMediaItems();
    currentRequest = new JSONObject();
    publish();
    stopSelf();
  }

  void cancelContextJobs() {
    Long[] jobs = contextJobs.toArray(new Long[0]);
    contextJobs.removeAll(Arrays.asList(jobs));
    for (long id : jobs) NativeBridge.cancel(id);
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
              .put(
                  "title",
                  !streamTitle.isEmpty()
                      ? streamTitle
                      : media == null ? "" : media.optString("title"))
              .put("stream_title", streamTitle)
              .put("stream_metadata_updates", streamMetadataUpdates)
              .put(
                  "station",
                  isLive() && !streamTitle.isEmpty() && media != null
                      ? media.optString("title")
                      : "")
              .put("playing", player.isPlaying())
              .put("playback_state", player.getPlaybackState())
              .put("play_when_ready", player.getPlayWhenReady())
              .put("suppression", player.getPlaybackSuppressionReason())
              .put("paused", pausedByUser)
              .put("navigation_pending", navigationPending || queuedNext > 0)
              .put("next_preloaded", PlaybackCache.ready)
              .put("start_ms", startMs)
              .put("first_frame_ms", firstFrameMs)
              .put("tap_to_output_ms", StartupTimings.total(startupToken))
              .put("buffering", player.getPlaybackState() == Player.STATE_BUFFERING)
              .put("buffered_ms", player.getTotalBufferedDuration())
              .put("position", player.getCurrentPosition() / 1000.0)
              .put("duration", Math.max(0, player.getDuration()) / 1000.0)
              .put("video", !currentRequest.optBoolean("audio", true))
              .put(
                  "offline",
                  currentRequest.optBoolean("offline") || currentRequest.optBoolean("local"))
              .put("quality", currentRequest.optString("quality", "1080"))
              .put("video_width", player.getVideoSize().width)
              .put("video_height", player.getVideoSize().height)
              .put("captions", captionText)
              .put("segments", sponsors)
              .put("volume", prefs.getFloat("volume", 0.7f))
              .put("actual_volume", player.getVolume())
              .put("preview", currentRequest.optBoolean("preview"))
              .put("speed", player.getPlaybackParameters().speed)
              .put("error", error.isEmpty() ? JSONObject.NULL : error)
              .put("error_trace", errorTrace);
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
        if (group.getType() != C.TRACK_TYPE_AUDIO
            && group.getType() != C.TRACK_TYPE_VIDEO
            && group.getType() != C.TRACK_TYPE_TEXT) continue;
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
      MediaBridge.notifyUi();
    } catch (JSONException ignored) {
    }
  }

  @Override
  public void onDestroy() {
    NativeBridge.activateContext("");
    cancelContextJobs();
    PlaybackCache.cancel();
    current = null;
    handler.removeCallbacksAndMessages(null);
    worker.shutdownNow();
    session.release();
    player.release();
    MediaBridge.playerSnapshot = "{}";
    super.onDestroy();
  }
}
