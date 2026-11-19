//! Behaviour tests for the Host Management window (#58).

use std::time::Instant;

use telepad_core::config::HostEntry;

use crate::hostsync::*;
use crate::{Effect, Msg, State, update};

const CONFIG: &str = "# laptop\n\nHost alpha\n  HostName 192.0.2.1\n  User ops\n  Port 22\n\nHost bravo\n  HostName 192.0.2.2\n  Port 22\n\nHost delta\n  HostName 192.0.2.4\n  Port 22\n\nHost shared1 shared2\n  User team\n\nHost *\n  ServerAliveInterval 30\n";

fn host(id: i64, alias: &str, ip: &str, user: &str) -> HostEntry {
    HostEntry {
        id: Some(id),
        hostname: ip.into(),
        user: user.into(),
        ..HostEntry::new(alias)
    }
}

/// Telepad: alpha (same), bravo (different user), charlie (only Telepad),
/// shared1 (on a shared line in the config). Config also has delta.
fn opened() -> State {
    let mut s = State::default();
    update(
        &mut s,
        Msg::HostsLoaded {
            hosts: Ok(vec![
                host(1, "alpha", "192.0.2.1", "ops"),
                host(2, "bravo", "192.0.2.2", "root"),
                host(3, "charlie", "192.0.2.3", ""),
                host(4, "shared1", "192.0.2.5", ""),
            ]),
            ssh_config_error: None,
        },
        Instant::now(),
    );
    let fx = send(&mut s, HsMsg::Open);
    assert!(matches!(
        &fx[..],
        [HsEffect::Load {
            path: None,
            remember: false
        }]
    ));
    send(
        &mut s,
        HsMsg::Loaded {
            path: "/home/me/.ssh/config".into(),
            result: Ok(CONFIG.into()),
        },
    );
    s
}

fn send(s: &mut State, m: HsMsg) -> Vec<HsEffect> {
    update(s, Msg::HostSync(m), Instant::now())
        .into_iter()
        .filter_map(|e| match e {
            Effect::HostSync(h) => Some(h),
            _ => None,
        })
        .collect()
}

fn hs(s: &State) -> &HostSync {
    s.model.host_sync.as_ref().unwrap()
}

fn states(s: &State) -> Vec<(String, telepad_core::hostsync::RowState)> {
    hs(s)
        .rows
        .iter()
        .map(|r| (r.alias.clone(), r.state))
        .collect()
}

fn apply_effect(s: &mut State) -> (Option<String>, Vec<HostEntry>, Vec<i64>) {
    match &send(s, HsMsg::Apply)[..] {
        [
            HsEffect::Apply {
                path,
                expected,
                config_text,
                upserts,
                deletes,
            },
        ] => {
            assert_eq!(path, "/home/me/.ssh/config");
            assert_eq!(expected, CONFIG, "Apply checks the file is unchanged");
            (config_text.clone(), upserts.clone(), deletes.clone())
        }
        other => panic!("{other:?} {:?}", hs(s).error),
    }
}

#[test]
fn shows_both_sides_matched_by_alias() {
    use telepad_core::hostsync::RowState::*;
    let s = opened();
    assert_eq!(
        states(&s),
        vec![
            ("alpha".into(), Same),
            ("bravo".into(), Different),
            ("charlie".into(), OnlyTelepad),
            ("shared1".into(), Different),
            ("shared2".into(), OnlyConfig),
            ("delta".into(), OnlyConfig),
        ]
    );
    assert_eq!(hs(&s).rows[1].diff, vec!["User"]);
    assert!(hs(&s).rows[3].cfg_read_only);
}

#[test]
fn copies_one_by_one_and_applies_both_sides() {
    use telepad_core::hostsync::RowState::*;
    let mut s = opened();
    send(&mut s, HsMsg::ToConfig(vec!["charlie".into()]));
    send(
        &mut s,
        HsMsg::ToTelepad(vec!["bravo".into(), "delta".into()]),
    );
    send(&mut s, HsMsg::ToConfig(vec!["shared1".into()]));
    assert!(
        hs(&s)
            .error
            .as_deref()
            .unwrap()
            .contains("shared Host line")
    );
    assert!(
        states(&s)
            .iter()
            .filter(|(a, _)| !a.starts_with("shared"))
            .all(|(_, st)| *st == Same)
    );
    assert_eq!(hs(&s).pending_count(), 3);

    send(&mut s, HsMsg::Preview);
    let p = hs(&s).preview.clone().unwrap();
    assert!(p.diff.iter().any(|(c, l)| *c == '+' && l == "Host charlie"));
    assert_eq!(
        p.telepad,
        vec!["update bravo (192.0.2.2)", "add delta (192.0.2.4)"]
    );

    let (text, upserts, deletes) = apply_effect(&mut s);
    let text = text.unwrap();
    assert!(text.contains("Host charlie") && text.contains("Host shared1 shared2"));
    assert!(text.find("Host charlie").unwrap() < text.find("Host *").unwrap());
    assert_eq!(
        upserts
            .iter()
            .map(|h| (h.alias.as_str(), h.id))
            .collect::<Vec<_>>(),
        vec![("bravo", Some(2)), ("delta", None)]
    );
    assert!(deletes.is_empty());
    assert!(send(&mut s, HsMsg::Apply).is_empty(), "one Apply at a time");

    // Failure keeps the changes; success reloads both sides.
    send(&mut s, HsMsg::Applied(Err("disk full".into())));
    assert_eq!(hs(&s).pending_count(), 3);
    let all = update(
        &mut s,
        Msg::HostSync(HsMsg::Applied(Ok("Saved".into()))),
        Instant::now(),
    );
    assert!(all.iter().any(|e| matches!(e, Effect::LoadHosts)));
    assert!(
        all.iter()
            .any(|e| matches!(e, Effect::HostSync(HsEffect::Load { .. })))
    );
    assert_eq!(hs(&s).pending_count(), 0);
    let _ = Same;
}

#[test]
fn merge_and_replace_in_both_directions() {
    let mut s = opened();
    send(
        &mut s,
        HsMsg::Bulk {
            from: Side::Telepad,
            mode: Mode::Merge,
        },
    );
    let (text, upserts, _) = apply_effect(&mut s);
    let text = text.unwrap();
    assert!(
        text.contains("Host charlie") && text.contains("Host delta"),
        "merge keeps delta"
    );
    assert!(text.contains("Host bravo\n  HostName 192.0.2.2\n  User root"));
    assert!(upserts.is_empty());

    let mut s = opened();
    send(
        &mut s,
        HsMsg::Bulk {
            from: Side::Telepad,
            mode: Mode::Replace,
        },
    );
    let (text, _, _) = apply_effect(&mut s);
    let text = text.unwrap();
    assert!(
        !text.contains("Host delta"),
        "replace removes config-only hosts"
    );
    assert!(
        text.contains("Host shared1 shared2") && text.contains("Host *"),
        "kept blocks stay"
    );

    let mut s = opened();
    send(
        &mut s,
        HsMsg::Bulk {
            from: Side::Config,
            mode: Mode::Replace,
        },
    );
    let (text, upserts, deletes) = apply_effect(&mut s);
    assert_eq!(text, None, "the config does not change");
    assert_eq!(
        upserts.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(),
        vec!["bravo", "delta", "shared1", "shared2"],
        "shared-line aliases are copied to Telepad"
    );
    let mut deletes = deletes;
    deletes.sort();
    assert_eq!(deletes, vec![3], "charlie leaves Telepad");
}

#[test]
fn sync_waits_for_a_side_on_differences() {
    let mut s = opened();
    send(&mut s, HsMsg::Sync);
    assert!(hs(&s).conflicts.contains("bravo"));
    assert!(send(&mut s, HsMsg::Apply).is_empty());
    send(&mut s, HsMsg::Preview);
    assert!(hs(&s).preview.is_none());
    assert!(hs(&s).error.as_deref().unwrap().contains("Choose a side"));
    send(&mut s, HsMsg::ToConfig(vec!["bravo".into()]));
    // shared1 is on a shared line: only ← can resolve it.
    send(&mut s, HsMsg::ToConfig(vec!["shared1".into()]));
    assert!(hs(&s).conflicts.contains("shared1"));
    send(&mut s, HsMsg::ToTelepad(vec!["shared1".into()]));
    assert!(hs(&s).conflicts.is_empty());
    let (text, upserts, _) = apply_effect(&mut s);
    let text = text.unwrap();
    assert!(text.contains("User root") && text.contains("Host charlie"));
    assert_eq!(
        upserts.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(),
        vec!["delta", "shared1", "shared2"]
    );
}

#[test]
fn edit_add_delete_and_undo() {
    let mut s = opened();
    // Rename in Telepad keeps the database row.
    send(
        &mut s,
        HsMsg::Edit {
            side: Side::Telepad,
            alias: "charlie".into(),
        },
    );
    let f = s.drafts.host_sync.form.as_mut().unwrap();
    assert_eq!(f.hostname, "192.0.2.3");
    f.alias = "charlie2".into();
    send(&mut s, HsMsg::FormSave);
    assert!(s.drafts.host_sync.form.is_none());
    // Add to the config, duplicate refused, bad port refused.
    send(&mut s, HsMsg::Add(Side::Config));
    let f = s.drafts.host_sync.form.as_mut().unwrap();
    f.alias = "delta".into();
    f.hostname = "192.0.2.9".into();
    send(&mut s, HsMsg::FormSave);
    assert!(
        s.drafts
            .host_sync
            .form
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("already exists")
    );
    let f = s.drafts.host_sync.form.as_mut().unwrap();
    f.alias = "echo".into();
    f.port = "x".into();
    send(&mut s, HsMsg::FormSave);
    assert!(s.drafts.host_sync.form.as_ref().unwrap().error.is_some());
    s.drafts.host_sync.form.as_mut().unwrap().port = "2200".into();
    send(&mut s, HsMsg::FormSave);
    // Rename in the config: old block deleted, new one added.
    send(
        &mut s,
        HsMsg::Edit {
            side: Side::Config,
            alias: "alpha".into(),
        },
    );
    s.drafts.host_sync.form.as_mut().unwrap().alias = "alpha1".into();
    send(&mut s, HsMsg::FormSave);
    send(
        &mut s,
        HsMsg::Delete {
            side: Side::Telepad,
            aliases: vec!["bravo".into()],
        },
    );
    send(
        &mut s,
        HsMsg::Delete {
            side: Side::Config,
            aliases: vec!["delta".into()],
        },
    );

    let (text, upserts, deletes) = apply_effect(&mut s);
    let text = text.unwrap();
    assert!(
        text.contains("Host echo\n  Port 2200") || text.contains("Host echo\n  HostName"),
        "{text}"
    );
    assert!(text.contains("Host alpha1") && !text.contains("Host alpha\n"));
    assert!(!text.contains("Host delta"));
    assert_eq!(
        upserts
            .iter()
            .map(|h| (h.alias.as_str(), h.id))
            .collect::<Vec<_>>(),
        vec![("charlie2", Some(3))]
    );
    assert_eq!(deletes, vec![2]);

    send(&mut s, HsMsg::Applied(Err("stopped for the test".into())));
    send(&mut s, HsMsg::Undo(vec!["charlie2".into(), "bravo".into()]));
    let (_, upserts, deletes) = apply_effect(&mut s);
    assert!(upserts.is_empty() && deletes.is_empty());
    send(&mut s, HsMsg::Discard);
    assert_eq!(hs(&s).pending_count(), 0);
}

#[test]
fn choosing_another_file_waits_for_pending_changes() {
    let mut s = opened();
    send(&mut s, HsMsg::ToConfig(vec!["charlie".into()]));
    assert!(send(&mut s, HsMsg::Picked("/tmp/other".into())).is_empty());
    assert!(send(&mut s, HsMsg::Reload).is_empty());
    send(&mut s, HsMsg::Discard);
    assert!(matches!(
        &send(&mut s, HsMsg::Picked("/tmp/other".into()))[..],
        [HsEffect::Load { path: Some(p), remember: true }] if p == "/tmp/other"
    ));
    send(
        &mut s,
        HsMsg::Loaded {
            path: "/tmp/other/config".into(),
            result: Err("Cannot read".into()),
        },
    );
    assert!(hs(&s).rows.is_empty() && hs(&s).error.is_some());
    send(&mut s, HsMsg::Close);
    assert!(s.model.host_sync.is_none());
}
