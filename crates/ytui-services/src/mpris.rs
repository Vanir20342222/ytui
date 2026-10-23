//! Desktop controls bridge to the existing player and application queue.
use crate::player::{PlayerCommand, PlayerStatus};
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tokio::sync::{mpsc, watch};
use zbus::{
    fdo,
    zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value},
};
const PATH: &str = "/org/mpris/MediaPlayer2";
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Next,
    Previous,
    Play,
    Quit,
    Stop,
}
#[derive(Clone)]
struct Controls {
    commands: mpsc::Sender<PlayerCommand>,
    events: mpsc::Sender<Event>,
    status: watch::Receiver<PlayerStatus>,
    queue: watch::Receiver<QueueCapabilities>,
}
#[derive(Clone, Copy, Default, PartialEq)]
struct QueueCapabilities {
    play: bool,
    next: bool,
    previous: bool,
}
fn busy() -> fdo::Error {
    fdo::Error::Failed("ytui is busy; retry the control".into())
}
impl Controls {
    fn command(&self, command: PlayerCommand) -> fdo::Result<()> {
        self.commands.try_send(command).map_err(|_| busy())
    }
    fn event(&self, event: Event) -> fdo::Result<()> {
        self.events.try_send(event).map_err(|_| busy())
    }
}
fn track_id(status: &PlayerStatus) -> OwnedObjectPath {
    let id = status
        .media
        .as_ref()
        .map(|media| hex::encode(Sha256::digest(media.url.as_bytes())))
        .unwrap_or_else(|| "none".into());
    OwnedObjectPath::try_from(format!("/org/mpris/MediaPlayer2/track/{id}"))
        .expect("hex is a valid object path")
}
#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Controls {
    fn raise(&self) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported(
            "ytui runs in your terminal".into(),
        ))
    }
    fn quit(&self) -> fdo::Result<()> {
        self.event(Event::Quit)
    }
    #[zbus(property)]
    fn can_quit(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> &str {
        "ytui"
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        "ytui"
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<&str> {
        vec![]
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<&str> {
        vec![]
    }
}
#[derive(Clone)]
struct Playback(Controls);
#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Playback {
    #[zbus(signal)]
    async fn seeked(context: &zbus::SignalContext<'_>, position: i64) -> zbus::Result<()>;
    fn next(&self) -> fdo::Result<()> {
        self.0.event(Event::Next)
    }
    fn previous(&self) -> fdo::Result<()> {
        self.0.event(Event::Previous)
    }
    fn pause(&self) -> fdo::Result<()> {
        self.0.command(PlayerCommand::SetPaused(true))
    }
    fn play(&self) -> fdo::Result<()> {
        if self.0.status.borrow().active {
            self.0.command(PlayerCommand::SetPaused(false))
        } else {
            self.0.event(Event::Play)
        }
    }
    fn play_pause(&self) -> fdo::Result<()> {
        if self.0.status.borrow().active {
            self.0
                .command(PlayerCommand::SetPaused(!self.0.status.borrow().paused))
        } else {
            self.0.event(Event::Play)
        }
    }
    fn stop(&self) -> fdo::Result<()> {
        self.0.event(Event::Stop)
    }
    fn seek(&self, offset: i64) -> fdo::Result<()> {
        self.0
            .command(PlayerCommand::Seek(offset as f64 / 1_000_000.0))
    }
    fn set_position(&self, track: ObjectPath<'_>, position: i64) -> fdo::Result<()> {
        if position >= 0 && track.as_str() == track_id(&self.0.status.borrow()).as_str() {
            self.0
                .command(PlayerCommand::SeekTo(position as f64 / 1_000_000.0))
        } else {
            Ok(())
        }
    }
    fn open_uri(&self, _uri: &str) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported("Choose a video in ytui".into()))
    }
    #[zbus(property)]
    fn playback_status(&self) -> &str {
        let state = self.0.status.borrow();
        if !state.active {
            "Stopped"
        } else if state.paused {
            "Paused"
        } else {
            "Playing"
        }
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        self.0.status.borrow().speed.max(0.25)
    }
    #[zbus(property)]
    fn set_rate(&self, rate: f64) -> zbus::Result<()> {
        if !rate.is_finite() || !(0.25..=4.0).contains(&rate) {
            return Err(fdo::Error::InvalidArgs("Rate must be 0.25–4".into()).into());
        }
        self.0
            .command(PlayerCommand::SetRate(rate))
            .map_err(Into::into)
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let status = self.0.status.borrow();
        let mut values = HashMap::new();
        values.insert(
            "mpris:trackid".into(),
            Value::from(track_id(&status)).try_into().unwrap(),
        );
        if let Some(media) = &status.media {
            values.insert(
                "xesam:title".into(),
                Value::from(status.title.clone()).try_into().unwrap(),
            );
            values.insert(
                "xesam:artist".into(),
                Value::from(vec![media.creator.clone()]).try_into().unwrap(),
            );
        }
        if let Some(duration) = status
            .duration
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            values.insert(
                "mpris:length".into(),
                OwnedValue::from((duration * 1_000_000.0) as i64),
            );
        }
        values
    }
    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.0.status.borrow().volume / 100.0
    }
    #[zbus(property)]
    fn set_volume(&self, volume: f64) -> zbus::Result<()> {
        if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
            return Err(fdo::Error::InvalidArgs("Volume must be 0–1".into()).into());
        }
        self.0
            .command(PlayerCommand::SetVolume(volume * 100.0))
            .map_err(Into::into)
    }
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        (self.0.status.borrow().position * 1_000_000.0) as i64
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        0.25
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        4.0
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.0.queue.borrow().next
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.0.queue.borrow().previous
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.0.queue.borrow().play
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.0.status.borrow().active
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.0.status.borrow().active
            && self
                .0
                .status
                .borrow()
                .media
                .as_ref()
                .is_some_and(|media| !media.live)
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}
pub struct Bridge {
    pub events: mpsc::Receiver<Event>,
    _connection: zbus::Connection,
    watcher: tokio::task::JoinHandle<()>,
    queue: watch::Sender<QueueCapabilities>,
}
impl Bridge {
    pub fn update_queue(&self, queue: &ytui_core::PlaybackQueue) {
        let capability = QueueCapabilities {
            play: !queue.items.is_empty(),
            next: queue.items.len() > 1
                && (queue.shuffle
                    || queue.repeat == ytui_core::Repeat::All
                    || queue
                        .current
                        .is_none_or(|index| index + 1 < queue.items.len())),
            previous: !queue.items.is_empty()
                && queue
                    .current
                    .is_some_and(|index| index > 0 || queue.repeat == ytui_core::Repeat::All),
        };
        self.queue.send_if_modified(|current| {
            if *current == capability {
                false
            } else {
                *current = capability;
                true
            }
        });
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.watcher.abort();
    }
}
pub async fn connect(
    commands: mpsc::Sender<PlayerCommand>,
    mut status: watch::Receiver<PlayerStatus>,
) -> Result<Bridge> {
    let (events, receiver) = mpsc::channel(16);
    let (queue, mut queue_status) = watch::channel(QueueCapabilities::default());
    let controls = Controls {
        commands,
        events,
        status: status.clone(),
        queue: queue_status.clone(),
    };
    let connection = zbus::connection::Builder::session()?
        .name("org.mpris.MediaPlayer2.ytui")?
        .serve_at(PATH, controls.clone())?
        .serve_at(PATH, Playback(controls))?
        .build()
        .await?;
    let signal_connection = connection.clone();
    let mut previous = status.borrow().clone();
    let watcher = tokio::spawn(async move {
        loop {
            let (changed, queue_changed) = tokio::select! { result = status.changed() => (result, false), result = queue_status.changed() => (result, true) };
            if changed.is_err() {
                break;
            }
            let current = status.borrow_and_update().clone();
            let Ok(interface) = signal_connection
                .object_server()
                .interface::<_, Playback>(PATH)
                .await
            else {
                break;
            };
            let player = interface.get().await;
            let context = interface.signal_context();
            if queue_changed {
                queue_status.borrow_and_update();
                let _ = player.can_go_next_changed(context).await;
                let _ = player.can_go_previous_changed(context).await;
                let _ = player.can_play_changed(context).await;
            }
            if current.active != previous.active || current.paused != previous.paused {
                let _ = player.playback_status_changed(context).await;
                let _ = player.can_pause_changed(context).await;
                let _ = player.can_seek_changed(context).await;
            }
            if current.media != previous.media
                || current.title != previous.title
                || current.duration != previous.duration
            {
                let _ = player.metadata_changed(context).await;
                let _ = player.can_play_changed(context).await;
                let _ = player.can_seek_changed(context).await;
            }
            if current.volume != previous.volume {
                let _ = player.volume_changed(context).await;
            }
            if current.speed != previous.speed {
                let _ = player.rate_changed(context).await;
            }
            if current.media == previous.media && (current.position - previous.position).abs() > 2.0
            {
                let _ = Playback::seeked(context, (current.position * 1_000_000.0) as i64).await;
            }
            previous = current;
        }
    });
    Ok(Bridge {
        events: receiver,
        _connection: connection,
        watcher,
        queue,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn absolute_desktop_controls_validate_values_and_do_not_start_another_player() {
        let (commands, mut received) = mpsc::channel(8);
        let (events, mut queue) = mpsc::channel(8);
        let (_, status) = watch::channel(PlayerStatus {
            active: true,
            paused: true,
            volume: 37.0,
            ..Default::default()
        });
        let player = Playback(Controls {
            commands,
            events,
            status,
            queue: watch::channel(QueueCapabilities::default()).1,
        });
        player.next().unwrap();
        player.pause().unwrap();
        player.set_volume(0.22).unwrap();
        assert!(player.set_volume(f64::NAN).is_err());
        assert!(player.set_rate(99.0).is_err());
        assert_eq!(queue.recv().await, Some(Event::Next));
        assert!(matches!(
            received.recv().await,
            Some(PlayerCommand::SetPaused(true))
        ));
        assert!(
            matches!(received.recv().await, Some(PlayerCommand::SetVolume(value)) if value == 22.0)
        );
        assert!(received.try_recv().is_err());
    }
    #[tokio::test]
    #[ignore = "requires an isolated session bus: dbus-run-session -- cargo test live_desktop_mpris -- --ignored"]
    async fn live_desktop_mpris_routes_queue_and_volume_on_an_actual_private_bus() {
        let (commands, mut received) = mpsc::channel(8);
        let (sender, status) = watch::channel(PlayerStatus {
            volume: 37.0,
            media: Some(ytui_core::Media {
                title: "Fixture".into(),
                url: "https://stream.test/?sig=SYNTHETIC".into(),
                ..Default::default()
            }),
            ..Default::default()
        });
        let mut bridge = connect(commands, status).await.unwrap();
        let mut playback = ytui_core::PlaybackQueue::default();
        playback.push(ytui_core::Media {
            title: "Queued".into(),
            ..Default::default()
        });
        bridge.update_queue(&playback);
        let connection = zbus::Connection::session().await.unwrap();
        let proxy = zbus::Proxy::new(
            &connection,
            "org.mpris.MediaPlayer2.ytui",
            PATH,
            "org.mpris.MediaPlayer2.Player",
        )
        .await
        .unwrap();
        assert_eq!(proxy.get_property::<f64>("Volume").await.unwrap(), 0.37);
        assert!(proxy.get_property::<bool>("CanPlay").await.unwrap());
        assert!(!proxy.get_property::<bool>("CanGoNext").await.unwrap());
        proxy.call::<_, _, ()>("Next", &()).await.unwrap();
        assert_eq!(bridge.events.recv().await, Some(Event::Next));
        proxy.set_property("Volume", 0.22f64).await.unwrap();
        assert!(
            matches!(received.recv().await, Some(PlayerCommand::SetVolume(value)) if value == 22.0)
        );
        let metadata: HashMap<String, OwnedValue> = proxy.get_property("Metadata").await.unwrap();
        assert!(!format!("{metadata:?}").contains("SYNTHETIC"));
        sender.send_modify(|status| {
            status.volume = 22.0;
            status.active = true;
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while proxy
                .get_property::<String>("PlaybackStatus")
                .await
                .unwrap()
                != "Playing"
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("property change signal was not delivered");
        drop(bridge);
    }
}
