use super::*;

#[test]
fn reads_are_not_changes_but_writes_and_errors_are() {
    let access = |kind| Ok(Event::new(EventKind::Access(kind)));
    assert!(!is_change(&access(AccessKind::Open(AccessMode::Read))));
    assert!(!is_change(&access(AccessKind::Close(AccessMode::Read))));
    assert!(!is_change(&access(AccessKind::Read)));
    assert!(is_change(&access(AccessKind::Close(AccessMode::Write))));
    assert!(is_change(&Ok(Event::new(EventKind::Create(
        notify::event::CreateKind::File
    )))));
    assert!(is_change(&Ok(Event::new(EventKind::Remove(
        notify::event::RemoveKind::Any
    )))));
    assert!(is_change(&Err(notify::Error::generic("queue overflow"))));
}

#[cfg(unix)]
#[test]
fn a_path_reached_through_a_link_below_the_root_is_told_apart() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("stick");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::create_dir(dir.path().join("busy")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("busy"), root.join("link")).unwrap();
    let roots = BTreeSet::from([root.clone()]);
    assert!(through_a_link(&root.join("link/log.txt"), &roots));
    assert!(through_a_link(&root.join("link/deep/x"), &roots));
    assert!(!through_a_link(&root.join("sub/a.txt"), &roots));
    // The link itself changing is the stick's own folder changing.
    assert!(!through_a_link(&root.join("link"), &roots));
    assert!(!through_a_link(&dir.path().join("elsewhere"), &roots));
}

#[tokio::test]
async fn the_operating_system_reports_a_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut events = NotifyEvents::start(dir.path()).unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/new.txt"), b"x").unwrap();
    let seen = tokio::time::timeout(Duration::from_secs(5), events.changed()).await;
    assert_eq!(seen, Ok(Some(())));
}

#[tokio::test]
async fn reading_a_file_is_not_reported() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
    let mut events = NotifyEvents::start(dir.path()).unwrap();
    let _ = std::fs::read(dir.path().join("a.txt")).unwrap();
    let seen = tokio::time::timeout(Duration::from_millis(500), events.changed()).await;
    assert!(seen.is_err(), "a read was reported as a change");
}

#[tokio::test]
async fn a_missing_folder_cannot_be_watched() {
    let dir = tempfile::tempdir().unwrap();
    let error = NotifyEvents::start(&dir.path().join("missing"))
        .err()
        .unwrap();
    assert!(error.to_string().contains("cannot watch"), "{error}");
}

#[tokio::test]
async fn periodic_events_come_every_interval() {
    // Measured from before the interval starts: its ticks count from
    // there, and a busy machine may take a while to come back.
    let started = tokio::time::Instant::now();
    let mut events = PeriodicEvents::new(Duration::from_millis(100));
    events.changed().await;
    events.changed().await;
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(200), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(1_000), "{elapsed:?}");
}

#[tokio::test]
async fn a_flood_of_signs_waits_as_one() {
    let mut events = PeriodicEvents::new(Duration::from_secs(3_600));
    let nudge = events.nudge();
    for _ in 0..100_000 {
        nudge.now();
    }
    events.changed().await;
    let more = tokio::time::timeout(Duration::from_millis(100), events.changed()).await;
    assert!(more.is_err(), "only one sign waited");
}

#[cfg(unix)]
#[test]
fn a_file_root_is_never_through_a_link_above_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("real")).unwrap();
    std::fs::write(dir.path().join("real/report.odt"), b"x").unwrap();
    std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("Documents")).unwrap();
    let root = dir.path().join("Documents/report.odt");
    assert!(!through_a_link(&root, &BTreeSet::from([root.clone()])));
}

/// Signs that come and go after the one awaited.
async fn quiet(events: &mut NotifyEvents) {
    while tokio::time::timeout(Duration::from_millis(300), events.changed())
        .await
        .is_ok()
    {}
}

#[tokio::test]
async fn a_file_sent_alone_is_seen_after_each_save_that_replaces_it() {
    // As the system names it: macOS reports /private/var, not /var.
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let own = base.join("own");
    std::fs::create_dir(&own).unwrap();
    let notes = base.join("notes.txt");
    std::fs::write(&notes, b"0").unwrap();
    std::fs::write(base.join("other.txt"), b"0").unwrap();
    let mut events = NotifyEvents::start(&own).unwrap();
    assert!(events
        .watch_roots(&[own.clone(), notes.clone()])
        .await
        .is_empty());
    for save in 1..=3 {
        // As most editors save: a new file, renamed over the old one.
        let new = base.join(".notes.txt.new");
        std::fs::write(&new, format!("{save}")).unwrap();
        std::fs::rename(&new, &notes).unwrap();
        let seen = tokio::time::timeout(Duration::from_secs(5), events.changed()).await;
        assert_eq!(seen, Ok(Some(())), "save {save} was not seen");
        quiet(&mut events).await;
    }
    // Another file in that folder is none of the stick's.
    std::fs::write(base.join("other.txt"), b"1").unwrap();
    let seen = tokio::time::timeout(Duration::from_millis(700), events.changed()).await;
    assert!(seen.is_err(), "a file beside the one sent was reported");
    // Watched no more once it is no root (on Windows the system lets go of
    // the folder a moment later).
    assert!(events
        .watch_roots(std::slice::from_ref(&own))
        .await
        .is_empty());
    tokio::time::sleep(Duration::from_millis(300)).await;
    quiet(&mut events).await;
    std::fs::write(&notes, b"after").unwrap();
    let seen = tokio::time::timeout(Duration::from_millis(700), events.changed()).await;
    assert!(
        seen.is_err(),
        "a file that is no root any more was reported"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_folder_moved_out_and_back_beside_a_file_sent_alone_is_watched_again() {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let own = base.join("own");
    let holidays = base.join("Holidays");
    let cover = base.join("cover.jpg");
    std::fs::create_dir(&own).unwrap();
    std::fs::create_dir(&holidays).unwrap();
    std::fs::write(&cover, b"0").unwrap();
    let roots = [own.clone(), holidays.clone(), cover.clone()];
    let mut events = NotifyEvents::start(&own).unwrap();
    assert!(events.watch_roots(&roots).await.is_empty());
    // Moved out and straight back (an undo), before the next look.
    std::fs::rename(&holidays, base.join("away")).unwrap();
    std::fs::rename(base.join("away"), &holidays).unwrap();
    let seen = tokio::time::timeout(Duration::from_secs(5), events.changed()).await;
    assert_eq!(seen, Ok(Some(())), "the move was not seen");
    quiet(&mut events).await;
    // The next look, as the watcher's loop makes one after its pass.
    assert!(events.watch_roots(&roots).await.is_empty());
    quiet(&mut events).await;
    std::fs::write(holidays.join("new.jpg"), b"x").unwrap();
    let seen = tokio::time::timeout(Duration::from_secs(5), events.changed()).await;
    assert_eq!(
        seen,
        Ok(Some(())),
        "a new file in the folder that came back was not seen"
    );
}

/// A build that makes the folder of a file sent alone again: the folder's
/// watch went with the old one, and the file is watched again at the next
/// look.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_file_sent_alone_is_watched_again_after_its_folder_is_made_again() {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let own = base.join("own");
    let dist = base.join("dist");
    let app = dist.join("app.zip");
    std::fs::create_dir(&own).unwrap();
    std::fs::create_dir(&dist).unwrap();
    std::fs::write(&app, b"0").unwrap();
    let roots = [own.clone(), app.clone()];
    let mut events = NotifyEvents::start(&own).unwrap();
    assert!(events.watch_roots(&roots).await.is_empty());
    std::fs::remove_dir_all(&dist).unwrap();
    std::fs::create_dir(&dist).unwrap();
    std::fs::write(&app, b"1").unwrap();
    let seen = tokio::time::timeout(Duration::from_secs(5), events.changed()).await;
    assert_eq!(seen, Ok(Some(())), "the new folder was not seen");
    quiet(&mut events).await;
    // The next look, as the watcher's loop makes one.
    assert!(events.watch_roots(&roots).await.is_empty());
    quiet(&mut events).await;
    std::fs::write(&app, b"2").unwrap();
    let seen = tokio::time::timeout(Duration::from_secs(5), events.changed()).await;
    assert_eq!(seen, Ok(Some(())), "a save in the new folder was not seen");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_file_in_a_folder_that_cannot_be_listed_is_still_watched() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let own = base.join("own");
    let locked = base.join("locked");
    let file = locked.join("report.odt");
    std::fs::create_dir(&own).unwrap();
    std::fs::create_dir(&locked).unwrap();
    std::fs::write(&file, b"0").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o311)).unwrap();
    let mut events = NotifyEvents::start(&own).unwrap();
    let errors = events.watch_roots(&[own.clone(), file.clone()]).await;
    // Written in place: what a watch of the file itself sees.
    let written = std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .and_then(|mut opened| opened.write_all(b"1"));
    let seen = tokio::time::timeout(Duration::from_secs(5), events.changed()).await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    written.unwrap();
    assert_eq!(seen, Ok(Some(())), "a write to the file was not seen");
}

/// A watch the system takes long to set up (notify walks the whole tree
/// first: minutes for a big folder on a slow disk) keeps no thread of the
/// share's, which can stop meanwhile (Quit, Stop sending). Here the walk is
/// a thread holding the watch for two seconds.
#[tokio::test]
async fn a_long_watch_leaves_the_share_free_to_stop() {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let own = base.join("own");
    let linked = base.join("linked");
    std::fs::create_dir(&own).unwrap();
    std::fs::create_dir(&linked).unwrap();
    let mut events = NotifyEvents::start(&own).unwrap();
    let watching = events.watching.clone();
    let (held, holding) = std::sync::mpsc::channel();
    let walk = std::thread::spawn(move || {
        let _walking = lock(&watching);
        held.send(()).unwrap();
        std::thread::sleep(Duration::from_secs(2));
    });
    holding.recv().unwrap();
    let started = std::time::Instant::now();
    let roots = [own.clone(), linked.clone()];
    let waited = tokio::time::timeout(Duration::from_millis(200), events.watch_roots(&roots)).await;
    assert!(waited.is_err(), "the watch is still being set up");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the wait was left at once"
    );
    walk.join().unwrap();
    // Once the walk is over, the roots are watched.
    assert!(events.watch_roots(&roots).await.is_empty());
    assert!(lock(&events.watching).watched.contains_key(&linked));
}
