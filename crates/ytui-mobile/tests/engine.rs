use serde_json::json;
use ytui_mobile::Engine;
fn engine() -> (Engine, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::new(&root.path().join("files"), &root.path().join("cache")).unwrap();
    (engine, root)
}
fn media(id: &str) -> ytui_core::Media {
    ytui_core::Media {
        url: format!("https://www.youtube.com/watch?v={id}"),
        title: format!("Fixture {id}"),
        ..Default::default()
    }
}
#[tokio::test]
async fn local_lyrics_keep_unicode_offsets_and_identity_after_restart_and_failed_writes() {
    let (e, root) = engine();
    let a = media("abcdefghijk");
    let b = media("lmnopqrstuv");
    let document = json!({"title":"Grüße 🎵","artist":"Artist","text":"[offset:-200]\n[00:01.234]Hello\n[00:04.567]World","offset_ms":500});
    let saved = e
        .request(json!({"op":"lyrics_save","url":a.url,"document":document}))
        .await
        .unwrap();
    assert_eq!(saved["track"]["lines"][0]["at_ms"], 1034);
    let restored = Engine::new(&root.path().join("files"), &root.path().join("cache")).unwrap();
    assert_eq!(
        restored
            .request(json!({"op":"lyrics_get","url":a.url}))
            .await
            .unwrap()["document"],
        document
    );
    assert!(
        restored
            .request(json!({"op":"lyrics_get","url":b.url}))
            .await
            .unwrap()
            .is_null()
    );
    for change in [
        json!({"offset_ms":120001}),
        json!({"text":"[offset:999999999999]\nHello"}),
        json!({"text":""}),
        json!({"text":"x".repeat(512*1024+1)}),
        json!({"artist":"x".repeat(513)}),
    ] {
        let mut bad = document.clone();
        for (k, v) in change.as_object().unwrap() {
            bad[k] = v.clone();
        }
        assert!(
            restored
                .request(json!({"op":"lyrics_save","url":a.url,"document":bad}))
                .await
                .is_err()
        );
        assert_eq!(
            restored
                .request(json!({"op":"lyrics_get","url":a.url}))
                .await
                .unwrap()["document"],
            document
        );
    }
    assert!(
        restored
            .request(json!({"op":"lyrics_save","url":"file:///tmp/invalid","document":document}))
            .await
            .is_err()
    );
    // Failed edits cannot publish an in-memory change that was never saved.
    // Resume/queue writes use a separate file to keep playback IO bounded.
    std::fs::remove_file(root.path().join("files/lyrics.json")).unwrap();
    std::fs::create_dir(root.path().join("files/lyrics.json")).unwrap();
    let mut changed = document.clone();
    changed["text"] = json!("Changed");
    assert!(
        restored
            .request(json!({"op":"lyrics_save","url":a.url,"document":changed}))
            .await
            .is_err()
    );
    assert_eq!(
        restored
            .request(json!({"op":"lyrics_get","url":a.url}))
            .await
            .unwrap()["document"],
        document
    );
}
#[tokio::test]
async fn local_lyrics_capacity_requires_explicit_removal_and_preserves_queue() {
    let (e, _root) = engine();
    let m = media("abcdefghijk");
    e.request(json!({"op":"enqueue","media":m})).await.unwrap();
    for i in 0..100 {
        e.request(json!({"op":"lyrics_save","url":format!("https://media.test/{i}"),"document":{"title":"Song","artist":"Artist","text":"Hello","offset_ms":0}})).await.unwrap();
    }
    let extra = json!({"op":"lyrics_save","url":m.url,"document":{"title":"Song","artist":"Artist","text":"Hello","offset_ms":0}});
    assert!(e.request(extra.clone()).await.is_err());
    e.request(json!({"op":"lyrics_remove","url":"https://media.test/0"}))
        .await
        .unwrap();
    e.request(extra).await.unwrap();
    e.request(json!({"op":"history_clear"})).await.unwrap();
    assert_eq!(
        e.request(json!({"op":"lyrics_get","url":m.url}))
            .await
            .unwrap()["document"]["text"],
        "Hello"
    );
    assert_eq!(
        e.request(json!({"op":"queue"})).await.unwrap()[0]["url"],
        m.url
    );
}
#[tokio::test]
async fn mobile_queue_saved_and_resume_are_durable_and_checked() {
    let (e, root) = engine();
    let m = media("abcdefghijk");
    e.request(json!({"op":"enqueue","media":m})).await.unwrap();
    e.request(json!({"op":"save_media","media":m}))
        .await
        .unwrap();
    e.request(json!({"op":"position","url":m.url,"milliseconds":12345}))
        .await
        .unwrap();
    let restored = Engine::new(&root.path().join("files"), &root.path().join("cache")).unwrap();
    assert_eq!(
        restored
            .request(json!({"op":"queue"}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        restored
            .request(json!({"op":"position","url":m.url}))
            .await
            .unwrap()["milliseconds"],
        12345
    );
    assert!(
        restored
            .request(json!({"op":"queue_remove","index":9}))
            .await
            .is_err()
    );
    assert!(
        restored
            .request(json!({"op":"position","url":m.url,"milliseconds":u64::MAX}))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn queue_reordering_is_durable_and_stale_actions_cannot_remove_the_wrong_item() {
    let (e, root) = engine();
    let a = media("abcdefghijk");
    let b = media("lmnopqrstuv");
    let c = media("zyxwvutsrqp");
    for item in [&a, &b, &c] {
        e.request(json!({"op":"enqueue","media":item}))
            .await
            .unwrap();
    }
    let changed = e
        .request(json!({"op":"queue_move","from":2,"to":0,"url":c.url}))
        .await
        .unwrap();
    assert_eq!(changed["items"][0]["url"], c.url);
    e.request(json!({"op":"queue_commit","url":c.url}))
        .await
        .unwrap();
    assert!(
        e.request(json!({"op":"queue_remove","index":1,"url":a.url}))
            .await
            .is_err()
    );
    assert!(
        e.request(json!({"op":"queue_move","from":0,"to":99,"url":a.url}))
            .await
            .is_err()
    );
    let restored = Engine::new(&root.path().join("files"), &root.path().join("cache")).unwrap();
    let queue = restored.request(json!({"op":"queue"})).await.unwrap();
    assert_eq!(queue[0]["url"], a.url);
    assert_eq!(queue[1]["url"], b.url);
    // A failed persistence attempt must not move the visible in-memory queue.
    std::fs::remove_file(root.path().join("files/library.json")).unwrap();
    std::fs::create_dir(root.path().join("files/library.json")).unwrap();
    assert!(
        e.request(json!({"op":"queue_move","from":0,"to":1,"url":a.url}))
            .await
            .is_err()
    );
    assert_eq!(e.request(json!({"op":"queue"})).await.unwrap(), queue);
}
#[tokio::test]
async fn playback_history_commits_only_accepted_navigation_and_preserves_forward_tracks() {
    let (e, root) = engine();
    let a = media("abcdefghijk");
    let b = media("lmnopqrstuv");
    let c = media("zyxwvutsrqp");
    for m in [&a, &b, &c] {
        e.request(json!({"op":"history_record","media":m}))
            .await
            .unwrap();
    }
    let previous = e
        .request(json!({"op":"history_peek","direction":"previous"}))
        .await
        .unwrap();
    assert_eq!(previous["media"]["url"], b.url);
    // Merely resolving a previous item cannot move the durable cursor.
    assert_eq!(
        e.request(json!({"op":"history_peek","direction":"previous"}))
            .await
            .unwrap(),
        previous
    );
    e.request(json!({"op":"history_record","media":b,"index":previous["index"]}))
        .await
        .unwrap();
    assert_eq!(
        e.request(json!({"op":"history_peek","direction":"next"}))
            .await
            .unwrap()["media"]["url"],
        c.url
    );
    let d = media("newtrack123");
    e.request(json!({"op":"history_record","media":d}))
        .await
        .unwrap();
    let restored = Engine::new(&root.path().join("files"), &root.path().join("cache")).unwrap();
    let history = restored
        .request(json!({"op":"history_list"}))
        .await
        .unwrap();
    assert_eq!(history.as_array().unwrap().len(), 4);
    assert_eq!(history[0]["url"], d.url);
    assert!(
        history
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["url"] == c.url)
    );
    let mut config = e.request(json!({"op":"settings"})).await.unwrap();
    config["playback_history"] = json!(false);
    e.request(json!({"op":"save_settings","config":config}))
        .await
        .unwrap();
    assert_eq!(
        e.request(json!({"op":"history_record","media":a}))
            .await
            .unwrap()["recorded"],
        false
    );
    assert_eq!(
        e.request(json!({"op":"history_peek","direction":"previous"}))
            .await
            .unwrap(),
        json!({})
    );
    e.request(json!({"op":"enqueue","media":a})).await.unwrap();
    e.request(json!({"op":"position","url":a.url,"milliseconds":2000}))
        .await
        .unwrap();
    e.request(json!({"op":"history_clear"})).await.unwrap();
    assert_eq!(
        e.request(json!({"op":"history_list"})).await.unwrap(),
        json!([])
    );
    assert_eq!(
        e.request(json!({"op":"position","url":a.url}))
            .await
            .unwrap()["milliseconds"],
        0
    );
    assert_eq!(
        e.request(json!({"op":"queue"}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

async fn cliamp_context_fixture() -> (
    String,
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(stream.read_u8().await.unwrap());
            assert!(request.len() < 8192);
        }
        assert!(String::from_utf8_lossy(&request).contains("radio.cliamp.stream/library"));
        let _ = started.send(());
        let _ = wait.await;
        let body = json!({"name":"Offline fixture","tracks":[
            {"id":"first","title":"First","url":"https://media.test/first.mp3"},
            {"id":"second","title":"Second","url":"https://media.test/second.mp3"},
            {"id":"third","title":"Third","url":"https://media.test/third.mp3"}
        ]})
        .to_string();
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
    });
    (proxy, ready, release, server)
}

#[tokio::test]
async fn collection_tail_is_automatic_but_cannot_consume_or_replace_the_manual_queue() {
    let (e, _root) = engine();
    let (proxy, _ready, release, server) = cliamp_context_fixture().await;
    let mut config = e.request(json!({"op":"settings"})).await.unwrap();
    config["proxy"] = json!(proxy);
    e.request(json!({"op":"save_settings","config":config}))
        .await
        .unwrap();
    e.activate_context(Some("collection-one".into()));
    release.send(()).unwrap();
    let result=tokio::time::timeout(std::time::Duration::from_secs(3),e.request(json!({"op":"prepare_context","context_id":"collection-one","request":ytui_core::BrowseRequest::CliampTracks("http://radio.cliamp.stream/library".into()),"source_id":"first"}))).await.unwrap().unwrap();
    assert_eq!(result["count"], 2);
    server.await.unwrap();
    let manual = media("manual12345");
    e.request(json!({"op":"enqueue","media":manual}))
        .await
        .unwrap();
    assert_eq!(
        e.request(json!({"op":"queue_peek","context_id":"collection-one"}))
            .await
            .unwrap()["url"],
        manual.url
    );
    e.request(json!({"op":"queue_commit","url":manual.url}))
        .await
        .unwrap();
    assert_eq!(
        e.request(json!({"op":"queue_peek","context_id":"wrong"}))
            .await
            .unwrap(),
        json!({})
    );
    e.activate_context(Some("collection-one".into())); // Pausing/restoring cannot reset its tail.
    let next = e
        .request(json!({"op":"queue_peek","context_id":"collection-one"}))
        .await
        .unwrap();
    assert_eq!(next["url"], "https://media.test/second.mp3");
    assert_eq!(next["context_entry"], true);
    assert_eq!(e.request(json!({"op":"queue_commit","context_id":"collection-one","context_entry":true,"url":"https://wrong.test"})).await.unwrap()["committed"],false);
    e.request(json!({"op":"queue_commit","context_id":"collection-one","context_entry":true,"url":next["url"]})).await.unwrap();
    assert_eq!(
        e.request(json!({"op":"queue_peek","context_id":"collection-one"}))
            .await
            .unwrap()["url"],
        "https://media.test/third.mp3"
    );
    assert_eq!(
        e.request(json!({"op":"prepare_context","context_id":"collection-one"}))
            .await
            .unwrap()["count"],
        1
    );
    e.request(json!({"op":"disconnect"})).await.unwrap();
    assert_eq!(
        e.request(json!({"op":"queue_peek","context_id":"collection-one"}))
            .await
            .unwrap(),
        json!({})
    );
}

#[tokio::test]
async fn a_late_collection_response_cannot_restore_an_obsolete_playback_context() {
    let (e, _root) = engine();
    let e = std::sync::Arc::new(e);
    let (proxy, ready, release, server) = cliamp_context_fixture().await;
    let mut config = e.request(json!({"op":"settings"})).await.unwrap();
    config["proxy"] = json!(proxy);
    e.request(json!({"op":"save_settings","config":config}))
        .await
        .unwrap();
    e.activate_context(Some("old".into()));
    let pending = e.clone();
    let task = tokio::spawn(async move {
        pending.request(json!({"op":"prepare_context","context_id":"old","request":ytui_core::BrowseRequest::CliampTracks("http://radio.cliamp.stream/library".into()),"source_id":"first"})).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), ready)
        .await
        .unwrap()
        .unwrap();
    e.activate_context(Some("new".into()));
    release.send(()).unwrap();
    assert_eq!(task.await.unwrap().unwrap()["prepared"], false);
    server.await.unwrap();
    assert_eq!(
        e.request(json!({"op":"prepare_context","context_id":"old"}))
            .await
            .unwrap()["prepared"],
        false
    );
    assert_eq!(
        e.request(json!({"op":"queue_peek","context_id":"new"}))
            .await
            .unwrap(),
        json!({})
    );
}

#[tokio::test]
async fn failed_disk_write_never_publishes_an_in_memory_queue_change() {
    let (e, root) = engine();
    std::fs::create_dir(root.path().join("files/library.json")).unwrap();
    assert!(
        e.request(json!({"op":"enqueue","media":media("abcdefghijk")}))
            .await
            .is_err()
    );
    assert_eq!(e.request(json!({"op":"queue"})).await.unwrap(), json!([]));
}
#[tokio::test]
async fn queue_peek_keeps_a_failed_playable_item_until_matching_commit() {
    let (e, _root) = engine();
    let a = media("abcdefghijk");
    let b = media("lmnopqrstuv");
    for m in [&a, &b] {
        e.request(json!({"op":"enqueue","media":m})).await.unwrap();
    }
    assert_eq!(
        e.request(json!({"op":"queue_peek"})).await.unwrap()["url"],
        a.url
    );
    e.request(json!({"op":"queue_commit","url":"https://wrong.test"}))
        .await
        .unwrap();
    assert_eq!(
        e.request(json!({"op":"queue"}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    e.request(json!({"op":"queue_commit","url":a.url}))
        .await
        .unwrap();
    assert_eq!(
        e.request(json!({"op":"queue_peek"})).await.unwrap()["url"],
        b.url
    );
}
#[tokio::test]
async fn settings_reject_invalid_modes_without_overwriting_previous_values() {
    let (e, _root) = engine();
    let old = e.request(json!({"op":"settings"})).await.unwrap();
    let mut bad = old.clone();
    bad["music_player_style"] = json!("unknown");
    assert!(
        e.request(json!({"op":"save_settings","config":bad}))
            .await
            .is_err()
    );
    assert_eq!(e.request(json!({"op":"settings"})).await.unwrap(), old);
    for name in ytui_core::theme::THEMES {
        let mut config = old.clone();
        config["theme"] = json!(name);
        e.request(json!({"op":"save_settings","config":config}))
            .await
            .unwrap();
        let theme = e.request(json!({"op":"theme","dark":false})).await.unwrap();
        assert!(theme["text"].is_array());
    }
}
#[tokio::test]
async fn an_old_import_cannot_reconnect_after_a_newer_disconnect_or_replace_a_new_account() {
    let (e, _root) = engine();
    let old = e.begin_account_change();
    let current = e.begin_account_change();
    assert!(
        e.request(
            json!({"op":"import_cookies","header":"SAPISID=synthetic-old","account_intent":old})
        )
        .await
        .is_err()
    );
    e.request(json!({"op":"import_cookies","header":"SAPISID=synthetic-current","account_intent":current})).await.unwrap();
    let disconnect = e.begin_account_change();
    e.request(json!({"op":"disconnect","account_intent":disconnect}))
        .await
        .unwrap();
    assert!(
        e.request(
            json!({"op":"import_cookies","header":"SAPISID=synthetic-old","account_intent":current})
        )
        .await
        .is_err()
    );
    assert!(
        !e.request(json!({"op":"account"})).await.unwrap()["connected"]
            .as_bool()
            .unwrap()
    );
}
#[tokio::test]
async fn unreadable_account_does_not_break_offline_library_or_expose_session_bytes() {
    let (e, root) = engine();
    let m = media("abcdefghijk");
    e.request(json!({"op":"save_media","media":m}))
        .await
        .unwrap();
    std::fs::write(
        root.path().join("files/account.json"),
        "{\"privateMarkerThatMustNeverBeReported\":",
    )
    .unwrap();
    let restored = Engine::new(&root.path().join("files"), &root.path().join("cache")).unwrap();
    let status = restored.request(json!({"op":"account"})).await.unwrap();
    assert_eq!(status["connected"], false);
    assert!(status["notice"].as_str().unwrap().contains("reconnect"));
    assert!(!status.to_string().contains("privateMarker"));
    assert_eq!(
        restored.request(json!({"op":"saved"})).await.unwrap()[0]["url"],
        m.url
    );
    assert!(
        root.path().join("files/account.json").exists(),
        "A read failure must not destroy the account document"
    );
    restored.request(json!({"op":"disconnect"})).await.unwrap();
    assert!(restored.request(json!({"op":"account"})).await.unwrap()["notice"].is_null());
}
#[tokio::test]
async fn authentication_import_is_scoped_and_disconnect_preserves_local_media() {
    let (e, root) = engine();
    assert!(
        e.request(json!({"op":"import_cookies","header":"SID=invalid"}))
            .await
            .is_err()
    );
    e.request(
        json!({"op":"import_cookies","header":"SAPISID=synthetic-session; SID=synthetic-only"}),
    )
    .await
    .unwrap();
    assert_eq!(
        e.request(json!({"op":"account"})).await.unwrap()["connected"],
        true
    );
    e.request(json!({"op":"save_media","media":media("abcdefghijk")}))
        .await
        .unwrap();
    e.request(json!({"op":"disconnect"})).await.unwrap();
    assert!(!root.path().join("files/account.json").exists());
    assert_eq!(
        e.request(json!({"op":"account"})).await.unwrap()["connected"],
        false
    );
    assert_eq!(
        e.request(json!({"op":"saved"}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[tokio::test]
async fn unknown_requests_and_oversized_media_fail_with_bounded_state() {
    let (e, _root) = engine();
    assert!(e.request(json!({"op":"unknown"})).await.is_err());
    let mut m = media("abcdefghijk");
    m.title = "x".repeat(300 * 1024);
    assert!(e.request(json!({"op":"enqueue","media":m})).await.is_err());
    assert_eq!(e.request(json!({"op":"queue"})).await.unwrap(), json!([]));
}

#[tokio::test]
async fn imported_palette_is_private_durable_and_invalid_import_does_not_replace_it() {
    let (e, root) = engine();
    let source = "background = '#101418'\nforeground = '#e5edf4'\naccent = '#6ddbc1'";
    let palette = e
        .request(json!({"op":"import_theme","text":source}))
        .await
        .unwrap();
    assert_eq!(palette["source"], "Omarchy");
    assert!(
        e.request(json!({"op":"import_theme","text":"invalid"}))
            .await
            .is_err()
    );
    let restored = Engine::new(&root.path().join("files"), &root.path().join("cache")).unwrap();
    assert_eq!(
        restored.request(json!({"op":"theme"})).await.unwrap(),
        palette
    );
    for proxy in [
        "socks5://localhost:9000",
        "http://user:password@localhost:9000",
    ] {
        let mut config = restored.request(json!({"op":"settings"})).await.unwrap();
        config["proxy"] = json!(proxy);
        assert!(
            restored
                .request(json!({"op":"save_settings","config":config}))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn disabled_sponsorblock_does_not_contact_the_api() {
    let (e, _root) = engine();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = e.request(json!({"op":"settings"})).await.unwrap();
    config["playback_sponsorblock"] = json!("off");
    config["proxy"] = json!(format!("http://{}", listener.local_addr().unwrap()));
    e.request(json!({"op":"save_settings","config":config}))
        .await
        .unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        e.request(json!({"op":"sponsor","media":media("abcdefghijk")})),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result, json!([]));
    assert!(listener.into_std().unwrap().accept().is_err());
}
