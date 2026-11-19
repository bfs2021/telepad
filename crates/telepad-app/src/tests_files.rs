//! Behaviour tests for the file windows (#62).

use std::time::Instant;

use telepad_core::config::HostEntry;
use telepad_core::files::transfer::{Decision, Direction, Event, Item};
use telepad_core::files::{Entry, Kind, local_join};

use crate::files::*;
use crate::{Effect, Msg, State, update};

fn state() -> State {
    let mut s = State::default();
    update(
        &mut s,
        Msg::HostsLoaded {
            hosts: Ok(vec![HostEntry {
                id: Some(1),
                hostname: "192.0.2.10".into(),
                ..HostEntry::new("web1")
            }]),
            ssh_config_error: None,
        },
        Instant::now(),
    );
    s
}

fn send(s: &mut State, m: FmMsg) -> Vec<FmEffect> {
    update(s, Msg::Files(m), Instant::now())
        .into_iter()
        .filter_map(|e| match e {
            Effect::Files(f) => Some(f),
            _ => None,
        })
        .collect()
}

fn fs(s: &State) -> &FileSession {
    &s.model.files[0]
}

fn draft(s: &mut State) -> &mut FileDraft {
    let id = s.model.files[0].id;
    s.drafts.files.get_mut(&id).unwrap()
}

fn entry(name: &str, dir: bool, size: u64, mtime: i64) -> Entry {
    Entry {
        name: name.into(),
        kind: if dir { Kind::Dir } else { Kind::File },
        size,
        mtime: Some(mtime),
        hidden: name.starts_with('.'),
        ..Default::default()
    }
}

/// Window open and both panes listed.
fn opened(mode: Mode) -> (State, FmId) {
    let mut s = state();
    let fx = send(&mut s, FmMsg::Open("web1".into()));
    assert!(matches!(&fx[..], [FmEffect::Connect { host, .. }] if host.alias == "web1"));
    assert!(
        send(&mut s, FmMsg::Open("web1".into())).is_empty(),
        "one window per host"
    );
    let id = fs(&s).id;
    let fx = send(
        &mut s,
        FmMsg::Connected {
            id,
            result: Ok((mode, "/home/ops".into(), None)),
            local_home: "/home/me".into(),
        },
    );
    let lists: Vec<(Side, String)> = fx
        .iter()
        .filter_map(|e| match e {
            FmEffect::List { side, path, .. } => Some((*side, path.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        lists,
        vec![
            (Side::Local, "/home/me".into()),
            (Side::Remote, "/home/ops".into())
        ]
    );
    send(
        &mut s,
        FmMsg::Listed {
            id,
            side: Side::Local,
            result: Ok((
                "/home/me".into(),
                vec![
                    entry("site", true, 0, 1),
                    entry("notes.txt", false, 5, 100),
                    entry(".bashrc", false, 1, 1),
                ],
            )),
        },
    );
    send(
        &mut s,
        FmMsg::Listed {
            id,
            side: Side::Remote,
            result: Ok((
                "/home/ops".into(),
                vec![entry("notes.txt", false, 9, 50), entry("logs", true, 0, 1)],
            )),
        },
    );
    (s, id)
}

fn item(label: &str, dir: bool, size: u64, exists: bool) -> Item {
    Item {
        src: format!("/home/me/{label}"),
        dst: format!("/home/ops/{label}"),
        label: label.into(),
        is_dir: dir,
        size,
        mtime: Some(10),
        perm: None,
        exists: exists.then_some((1, Some(5))),
    }
}

#[test]
fn opens_lists_navigates_and_remembers_folders() {
    let (mut s, id) = opened(Mode::Sftp);
    let local = fs(&s).local.sorted(false, "");
    assert_eq!(
        local.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        vec!["site", "notes.txt"]
    );
    assert_eq!(
        fs(&s).local.sorted(true, "").len(),
        3,
        "hidden files on request"
    );
    assert_eq!(
        fs(&s).local.sorted(true, "NOTE").len(),
        1,
        "filter ignores case"
    );
    send(
        &mut s,
        FmMsg::Sort {
            id,
            side: Side::Local,
            col: SortCol::Size,
        },
    );
    send(
        &mut s,
        FmMsg::Sort {
            id,
            side: Side::Local,
            col: SortCol::Size,
        },
    );
    assert!(fs(&s).local.descending);
    assert_eq!(
        fs(&s).local.sorted(false, "")[0].name,
        "site",
        "folders stay first"
    );

    let path = |fx: &[FmEffect]| match fx {
        [FmEffect::List { path, .. }] => path.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        path(&send(
            &mut s,
            FmMsg::Enter {
                id,
                side: Side::Remote,
                name: "logs".into()
            }
        )),
        "/home/ops/logs"
    );
    send(
        &mut s,
        FmMsg::Listed {
            id,
            side: Side::Remote,
            result: Ok(("/home/ops/logs".into(), vec![])),
        },
    );
    assert_eq!(
        path(&send(
            &mut s,
            FmMsg::Up {
                id,
                side: Side::Remote
            }
        )),
        "/home/ops"
    );
    assert_eq!(
        path(&send(
            &mut s,
            FmMsg::Home {
                id,
                side: Side::Remote
            }
        )),
        "/home/ops"
    );
    // A folder that cannot be opened keeps the last good one.
    send(
        &mut s,
        FmMsg::Navigate {
            id,
            side: Side::Remote,
            path: " /root ".into(),
        },
    );
    assert_eq!(draft(&mut s).remote_path, "/root");
    send(
        &mut s,
        FmMsg::Listed {
            id,
            side: Side::Remote,
            result: Err("Permission denied".into()),
        },
    );
    assert_eq!(fs(&s).remote.path, "/home/ops/logs");
    assert_eq!(draft(&mut s).remote_path, "/home/ops/logs");
    assert!(fs(&s).remote.error.is_some());

    let fx = send(&mut s, FmMsg::Close(id));
    assert!(
        matches!(&fx[0], FmEffect::SavePaths { alias, remote, .. } if alias == "web1" && remote == "/home/ops/logs")
    );
    assert!(matches!(fx.last(), Some(FmEffect::Close(_))));
    assert!(s.model.files.is_empty() && s.drafts.files.is_empty());
}

#[test]
fn upload_asks_about_existing_files_then_copies() {
    let (mut s, id) = opened(Mode::Sftp);
    let fx = send(
        &mut s,
        FmMsg::Copy {
            id,
            from: Side::Local,
            names: vec!["site".into(), "notes.txt".into()],
        },
    );
    let tid = match &fx[..] {
        [
            FmEffect::Plan {
                transfer,
                direction: Direction::Upload,
                sources,
                dest,
                ..
            },
        ] => {
            assert_eq!(dest, "/home/ops");
            assert_eq!(
                sources,
                &vec![
                    (local_join("/home/me", "site"), true),
                    (local_join("/home/me", "notes.txt"), false)
                ]
            );
            *transfer
        }
        other => panic!("{other:?}"),
    };
    let items = vec![
        item("site/", true, 0, false),
        item("site/a.txt", false, 100, true),
        item("site/b.txt", false, 200, true),
        item("notes.txt", false, 5, true),
    ];
    send(
        &mut s,
        FmMsg::Planned {
            id,
            transfer: tid,
            result: Ok(items),
        },
    );
    assert_eq!(fs(&s).transfer(tid).unwrap().state, TState::Asking);
    assert_eq!(fs(&s).ask.as_ref().unwrap().conflicts, vec![1, 2, 3]);
    assert!(
        send(
            &mut s,
            FmMsg::Decide {
                id,
                decision: Decision::Skip
            }
        )
        .is_empty()
    );
    assert_eq!(fs(&s).ask.as_ref().unwrap().at, 1);
    draft(&mut s).apply_all = true;
    let fx = send(
        &mut s,
        FmMsg::Decide {
            id,
            decision: Decision::Overwrite,
        },
    );
    match &fx[..] {
        [
            FmEffect::Run {
                decisions, items, ..
            },
        ] => {
            assert_eq!(decisions.get(&1), Some(&Decision::Skip));
            assert_eq!(decisions.get(&2), Some(&Decision::Overwrite));
            assert_eq!(decisions.get(&3), Some(&Decision::Overwrite));
            assert_eq!(items.len(), 4);
        }
        other => panic!("{other:?}"),
    }
    assert!(fs(&s).ask.is_none());
    assert!(!draft(&mut s).apply_all, "Apply to all is per question");
    let t = fs(&s).transfer(tid).unwrap();
    assert_eq!(
        (t.state.clone(), t.bytes_total),
        (TState::Running, 205),
        "skipped file not counted"
    );

    send(
        &mut s,
        FmMsg::Progress {
            id,
            transfer: tid,
            event: Event::Start(2),
        },
    );
    send(
        &mut s,
        FmMsg::Progress {
            id,
            transfer: tid,
            event: Event::Bytes(200),
        },
    );
    send(
        &mut s,
        FmMsg::Progress {
            id,
            transfer: tid,
            event: Event::Done(3, Err("Permission denied".into())),
        },
    );
    assert_eq!(fs(&s).transfer(tid).unwrap().current, "site/b.txt");
    assert_eq!(
        fs(&s).transfer(tid).unwrap().errors,
        vec!["notes.txt: Permission denied".to_string()]
    );
    let fx = send(
        &mut s,
        FmMsg::Finished {
            id,
            transfer: tid,
            result: Ok((2, 1, 1)),
        },
    );
    assert!(
        matches!(
            &fx[..],
            [FmEffect::List {
                side: Side::Remote,
                ..
            }]
        ),
        "target pane refreshed"
    );
    assert_eq!(
        fs(&s).transfer(tid).unwrap().state,
        TState::Done {
            copied: 2,
            skipped: 1,
            failed: 1
        }
    );
    send(&mut s, FmMsg::ClearFinished(id));
    assert!(fs(&s).transfers.is_empty());
}

#[test]
fn download_without_conflicts_runs_and_can_be_cancelled() {
    let (mut s, id) = opened(Mode::Sftp);
    let fx = send(
        &mut s,
        FmMsg::Copy {
            id,
            from: Side::Remote,
            names: vec!["logs".into()],
        },
    );
    let tid = match &fx[..] {
        [
            FmEffect::Plan {
                transfer,
                direction: Direction::Download,
                dest,
                ..
            },
        ] => {
            assert_eq!(dest, "/home/me");
            *transfer
        }
        other => panic!("{other:?}"),
    };
    assert!(matches!(
        &send(&mut s, FmMsg::Cancel { id, transfer: tid })[..],
        [FmEffect::Cancel { .. }]
    ));
    let fx = send(
        &mut s,
        FmMsg::Planned {
            id,
            transfer: tid,
            result: Ok(vec![item("logs/", true, 0, false)]),
        },
    );
    assert!(matches!(
        &fx[..],
        [FmEffect::Run {
            direction: Direction::Download,
            ..
        }]
    ));
    send(
        &mut s,
        FmMsg::Finished {
            id,
            transfer: tid,
            result: Err("Cancelled".into()),
        },
    );
    assert_eq!(fs(&s).transfer(tid).unwrap().state, TState::Cancelled);
    assert!(
        send(
            &mut s,
            FmMsg::Copy {
                id,
                from: Side::Remote,
                names: vec![]
            }
        )
        .is_empty()
    );
    // Asking can be cancelled without the runtime.
    let fx = send(
        &mut s,
        FmMsg::Copy {
            id,
            from: Side::Local,
            names: vec!["notes.txt".into()],
        },
    );
    let FmEffect::Plan { transfer, .. } = &fx[0] else {
        panic!()
    };
    let t2 = *transfer;
    send(
        &mut s,
        FmMsg::Planned {
            id,
            transfer: t2,
            result: Ok(vec![item("notes.txt", false, 5, true)]),
        },
    );
    send(&mut s, FmMsg::AskCancel(id));
    assert_eq!(fs(&s).transfer(t2).unwrap().state, TState::Cancelled);
    assert!(fs(&s).ask.is_none());
}

#[test]
fn hosts_without_sftp_copy_with_scp_after_a_name_check() {
    let (mut s, id) = opened(Mode::Fallback);
    let fx = send(
        &mut s,
        FmMsg::Copy {
            id,
            from: Side::Local,
            names: vec!["notes.txt".into(), "site".into()],
        },
    );
    assert!(fx.is_empty(), "notes.txt exists on the host: ask first");
    let tid = fs(&s).transfers[0].id;
    assert_eq!(fs(&s).ask.as_ref().unwrap().conflicts, vec![0]);
    let fx = send(
        &mut s,
        FmMsg::Decide {
            id,
            decision: Decision::NewerOnly,
        },
    );
    match &fx[..] {
        [
            FmEffect::Scp {
                sources,
                dest,
                direction: Direction::Upload,
                ..
            },
        ] => {
            // notes.txt (local 100) is newer than the host's (50): copied.
            assert_eq!(
                sources,
                &vec![
                    (local_join("/home/me", "notes.txt"), false),
                    (local_join("/home/me", "site"), true)
                ]
            );
            assert_eq!(dest, "/home/ops");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(fs(&s).transfer(tid).unwrap().state, TState::Running);
}

#[test]
fn rsync_previews_before_it_runs() {
    let (mut s, id) = opened(Mode::Sftp);
    send(&mut s, FmMsg::RsyncChecked { id, result: Ok(()) });
    draft(&mut s).use_rsync = true;
    let fx = send(
        &mut s,
        FmMsg::Copy {
            id,
            from: Side::Local,
            names: vec!["site".into()],
        },
    );
    let tid = match &fx[..] {
        [
            FmEffect::Rsync {
                transfer,
                dry_run: true,
                sources,
                dest,
                ..
            },
        ] => {
            assert_eq!(
                (sources.clone(), dest.as_str()),
                (vec![local_join("/home/me", "site")], "/home/ops")
            );
            *transfer
        }
        other => panic!("{other:?}"),
    };
    send(
        &mut s,
        FmMsg::RsyncPreviewed {
            id,
            transfer: tid,
            result: Ok(vec!["site/index.html".into()]),
        },
    );
    assert_eq!(
        fs(&s).transfer(tid).unwrap().state,
        TState::Previewed(vec!["site/index.html".into()])
    );
    let fx = send(&mut s, FmMsg::RsyncRun { id, transfer: tid });
    assert!(matches!(&fx[..], [FmEffect::Rsync { dry_run: false, .. }]));
    assert!(
        send(&mut s, FmMsg::RsyncRun { id, transfer: tid }).is_empty(),
        "runs once"
    );

    // Not available: the switch has no effect.
    send(
        &mut s,
        FmMsg::RsyncChecked {
            id,
            result: Err("rsync is not installed on this computer".into()),
        },
    );
    let fx = send(
        &mut s,
        FmMsg::Copy {
            id,
            from: Side::Local,
            names: vec!["site".into()],
        },
    );
    assert!(matches!(&fx[..], [FmEffect::Plan { .. }]));
}

#[test]
fn connection_errors_can_be_retried() {
    let mut s = state();
    send(&mut s, FmMsg::Open("web1".into()));
    let id = fs(&s).id;
    send(
        &mut s,
        FmMsg::Connected {
            id,
            result: Err("Permission denied (publickey)".into()),
            local_home: "/home/me".into(),
        },
    );
    assert!(fs(&s).error.is_some());
    let fx = send(&mut s, FmMsg::Retry(id));
    assert!(matches!(
        &fx[..],
        [FmEffect::Close(_), FmEffect::Connect { .. }]
    ));
    assert!(
        send(
            &mut s,
            FmMsg::Copy {
                id,
                from: Side::Local,
                names: vec!["x".into()]
            }
        )
        .is_empty()
    );
    let fx = send(&mut s, FmMsg::Close(id));
    assert!(
        matches!(&fx[..], [FmEffect::Close(_)]),
        "nothing to remember before it connected"
    );
}
