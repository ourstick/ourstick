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
    assert!(events.watch_roots(&[own.clone(), notes.clone()]).is_empty());
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
    // Watched no more once it is no root.
    assert!(events.watch_roots(std::slice::from_ref(&own)).is_empty());
    std::fs::write(&notes, b"after").unwrap();
    let seen = tokio::time::timeout(Duration::from_millis(700), events.changed()).await;
    assert!(
        seen.is_err(),
        "a file that is no root any more was reported"
    );
}
