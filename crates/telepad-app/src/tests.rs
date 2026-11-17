use std::time::{Duration, Instant};

use telepad_core::collector::Update;
use telepad_core::config::{Access, HostEntry, Inventory};
use telepad_core::model::HostReport;
use telepad_core::store::{ImportMode, ImportSummary, UiPrefs};
use telepad_core::update::{CHECK_EVERY, Check, UpdateConfig};

use crate::*;

fn host(id: i64, alias: &str) -> HostEntry {
    HostEntry {
        id: Some(id),
        hostname: format!("192.0.2.{id}"),
        ..HostEntry::new(alias)
    }
}

fn started(now: Instant) -> (State, Vec<Effect>) {
    let mut s = State::default();
    let fx = update(
        &mut s,
        Msg::Started {
            db_path: "telepad.db".into(),
            ssh_config_path: "telepad.ssh_config".into(),
            imported_from: None,
            update_config: Ok(UpdateConfig::default()),
            ui_prefs: UiPrefs::default(),
        },
        now,
    );
    (s, fx)
}

fn loaded(s: &mut State, hosts: Vec<HostEntry>, now: Instant) -> Vec<Effect> {
    update(
        s,
        Msg::HostsLoaded {
            hosts: Ok(hosts),
            ssh_config_error: None,
        },
        now,
    )
}

fn collect_aliases(fx: &[Effect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Collect(h) => Some(h.iter().map(|h| h.alias.clone())),
            _ => None,
        })
        .flatten()
        .collect()
}

fn check(version: &str, newer: bool) -> Check {
    Check {
        version: version.into(),
        notes: None,
        page_url: String::new(),
        asset: Some(telepad_core::update::Asset {
            name: "telepad.zip".into(),
            url: "https://github.com/x/y/releases/download/v1/telepad.zip".into(),
            size: 1,
            sha256_url: "https://github.com/x/y/releases/download/v1/telepad.zip.sha256".into(),
        }),
        newer,
    }
}

#[test]
fn start_loads_hosts_checks_updates_and_polls_everything_once() {
    let t0 = Instant::now();
    let (mut s, fx) = started(t0);
    assert!(matches!(fx[0], Effect::LoadHosts));
    assert!(fx.iter().any(|e| matches!(e, Effect::CheckUpdate(_))));
    assert_eq!(s.model.update.phase, UpdatePhase::Checking);

    let mut quiet = host(3, "nas01");
    quiet.unmonitored = true;
    let fx = loaded(&mut s, vec![host(1, "web01"), host(2, "db01"), quiet], t0);
    assert_eq!(collect_aliases(&fx), ["web01", "db01"]);
    assert_eq!(s.model.hosts.len(), 3);
    assert_eq!(s.model.last_refresh, Some(t0));
}

#[test]
fn fatal_and_bad_update_settings_are_reported() {
    let mut s = State::default();
    update(&mut s, Msg::Fatal("locked".into()), Instant::now());
    assert!(s.model.fatal.as_deref().unwrap().contains("locked"));

    let mut s = State::default();
    let fx = update(
        &mut s,
        Msg::Started {
            db_path: "a".into(),
            ssh_config_path: "b".into(),
            imported_from: Some("config".into()),
            update_config: Err("bad".into()),
            ui_prefs: UiPrefs {
                compact_host_list: false,
                group_by_status: true,
                overview_cards: true,
                theme: telepad_core::store::ThemePref::Dark,
            },
        },
        Instant::now(),
    );
    assert!(!fx.iter().any(|e| matches!(e, Effect::CheckUpdate(_))));
    assert!(s.model.update.error.as_deref().unwrap().contains("bad"));
    assert!(s.model.message.as_ref().unwrap().0.contains("Created"));
    assert!(
        !s.drafts.compact_host_list,
        "remembered view settings applied"
    );
    assert!(s.drafts.group_by_status);
    assert!(s.drafts.overview_cards);
    assert_eq!(s.drafts.theme, telepad_core::store::ThemePref::Dark);
}

#[test]
fn reload_keeps_results_and_selection_and_polls_only_changes() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    loaded(&mut s, vec![host(1, "web01"), host(2, "db01")], t0);
    s.model.hosts[0].report = Some(HostReport::default());
    s.model.hosts[1].report = Some(HostReport::default());
    update(&mut s, Msg::Select(Some(1)), t0);

    let mut changed = host(1, "web01");
    changed.port = 2222;
    let fx = loaded(&mut s, vec![host(3, "new01"), host(2, "db01"), changed], t0);

    assert_eq!(collect_aliases(&fx), ["new01", "web01"]);
    assert!(
        s.model.hosts[1].report.is_some(),
        "unchanged db01 keeps its report"
    );
    assert!(
        s.model.hosts[2].report.is_none(),
        "changed web01 starts over"
    );
    assert_eq!(
        s.model.selected,
        Some(1),
        "db01 stays selected at its new index"
    );
}

#[test]
fn auto_refresh_waits_for_interval_and_idle_collector() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    loaded(&mut s, vec![host(1, "web01")], t0);
    let at = |secs| t0 + Duration::from_secs(secs);

    assert!(collect_aliases(&update(&mut s, Msg::Tick { busy: 0 }, at(59))).is_empty());
    assert!(collect_aliases(&update(&mut s, Msg::Tick { busy: 1 }, at(61))).is_empty());
    assert_eq!(s.model.busy, 1);
    assert_eq!(
        collect_aliases(&update(&mut s, Msg::Tick { busy: 0 }, at(61))),
        ["web01"]
    );
    assert!(collect_aliases(&update(&mut s, Msg::Tick { busy: 0 }, at(62))).is_empty());

    s.drafts.auto_refresh = false;
    assert!(collect_aliases(&update(&mut s, Msg::Tick { busy: 0 }, at(500))).is_empty());
}

#[test]
fn update_check_repeats_every_six_hours_but_not_while_checking() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    let checks = |fx: &[Effect]| {
        fx.iter()
            .filter(|e| matches!(e, Effect::CheckUpdate(_)))
            .count()
    };

    assert_eq!(
        checks(&update(&mut s, Msg::Tick { busy: 0 }, t0 + CHECK_EVERY)),
        0,
        "still checking"
    );
    update(&mut s, Msg::UpdateChecked(Err("offline".into())), t0);
    assert_eq!(s.model.update.error.as_deref(), Some("offline"));
    assert!(s.model.message.is_none(), "automatic check fails quietly");
    assert_eq!(
        checks(&update(&mut s, Msg::Tick { busy: 0 }, t0 + CHECK_EVERY / 2)),
        0
    );
    assert_eq!(
        checks(&update(&mut s, Msg::Tick { busy: 0 }, t0 + CHECK_EVERY)),
        1
    );

    update(&mut s, Msg::UpdateChecked(Err("offline".into())), t0);
    let fx = update(&mut s, Msg::UpdateCheck, t0);
    assert_eq!(checks(&fx), 1, "manual check any time");
    update(&mut s, Msg::UpdateChecked(Ok(check("0.0.1", false))), t0);
    assert!(s.model.message.as_ref().unwrap().0.contains("up to date"));
}

#[test]
fn newer_release_banner_install_and_restart() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    update(&mut s, Msg::UpdateChecked(Ok(check("9.0.0", true))), t0);
    assert_eq!(
        s.model.update.banner().map(|c| c.version.as_str()),
        Some("9.0.0")
    );

    let fx = update(&mut s, Msg::UpdateInstall, t0);
    assert!(matches!(fx[..], [Effect::InstallUpdate(_)]));
    assert!(
        s.model.update.banner().is_none(),
        "no banner while installing"
    );
    assert!(
        update(&mut s, Msg::UpdateInstall, t0).is_empty(),
        "only one install at a time"
    );

    update(&mut s, Msg::UpdateInstalled(Ok("9.0.0".into())), t0);
    assert_eq!(s.model.update.phase, UpdatePhase::Installed("9.0.0".into()));
    assert!(matches!(
        update(&mut s, Msg::Restart, t0)[..],
        [Effect::Relaunch]
    ));
}

#[test]
fn failed_install_returns_to_idle_and_later_hides_banner() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    update(&mut s, Msg::UpdateChecked(Ok(check("9.0.0", true))), t0);
    update(&mut s, Msg::UpdateInstall, t0);
    update(
        &mut s,
        Msg::UpdateInstalled(Err("hash mismatch".into())),
        t0,
    );
    assert_eq!(s.model.update.phase, UpdatePhase::Idle);
    assert!(
        s.model
            .message
            .as_ref()
            .unwrap()
            .0
            .contains("hash mismatch")
    );

    update(&mut s, Msg::UpdateWindow(true), t0);
    update(&mut s, Msg::UpdateLater, t0);
    assert!(s.model.update.banner().is_none());
    assert!(!s.model.update.window);
}

#[test]
fn update_settings_are_validated_before_saving() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    update(&mut s, Msg::UpdateChecked(Err("x".into())), t0);
    s.drafts.update_form.repo = "not a repo".into();
    assert!(update(&mut s, Msg::UpdateSaveSettings, t0).is_empty());
    assert!(s.drafts.update_form_error.is_some());

    s.drafts.update_form.repo = "  someone/fork ".into();
    let fx = update(&mut s, Msg::UpdateSaveSettings, t0);
    let Effect::SaveUpdateConfig(c) = &fx[0] else {
        panic!("{fx:?}")
    };
    assert_eq!(c.repo, "someone/fork");

    let fx = update(&mut s, Msg::UpdateSettingsSaved(Ok(c.clone())), t0);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::CheckUpdate(c) if c.repo == "someone/fork"))
    );
    assert!(s.drafts.update_form_error.is_none());
}

#[test]
fn editor_save_validates_then_saves_reloads_and_selects() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    loaded(&mut s, vec![host(1, "web01")], t0);

    update(&mut s, Msg::AddHost, t0);
    s.drafts.editor.as_mut().unwrap().entry.alias = "bad alias".into();
    assert!(update(&mut s, Msg::EditorSave, t0).is_empty());
    assert!(s.drafts.editor.as_ref().unwrap().error.is_some());

    s.drafts.editor.as_mut().unwrap().entry.alias = "db01".into();
    let fx = update(&mut s, Msg::EditorSave, t0);
    assert!(matches!(&fx[..], [Effect::SaveHost(h)] if h.alias == "db01"));

    let fx = update(&mut s, Msg::HostSaved(Ok((7, "db01".into()))), t0);
    assert!(matches!(fx[..], [Effect::LoadHosts]));
    assert!(s.drafts.editor.is_none());
    loaded(&mut s, vec![host(1, "web01"), host(7, "db01")], t0);
    assert_eq!(s.model.selected, Some(1));

    update(&mut s, Msg::Edit(0), t0);
    update(&mut s, Msg::HostSaved(Err("alias exists".into())), t0);
    assert_eq!(
        s.drafts.editor.as_ref().unwrap().error.as_deref(),
        Some("alias exists"),
        "a failed save keeps the form open"
    );
}

#[test]
fn delete_needs_confirmation_and_clears_selection() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    loaded(&mut s, vec![host(1, "web01"), host(2, "db01")], t0);
    update(&mut s, Msg::Select(Some(1)), t0);

    assert!(update(&mut s, Msg::Delete(1), t0).is_empty());
    assert_eq!(s.model.pending_delete, Some((2, "db01".into())));
    update(&mut s, Msg::CancelDelete, t0);
    assert!(update(&mut s, Msg::ConfirmDelete, t0).is_empty());

    update(&mut s, Msg::Delete(1), t0);
    let fx = update(&mut s, Msg::ConfirmDelete, t0);
    assert!(matches!(&fx[..], [Effect::DeleteHost { id: 2, .. }]));
    let fx = update(
        &mut s,
        Msg::HostDeleted {
            id: 2,
            alias: "db01".into(),
            result: Ok(()),
        },
        t0,
    );
    assert!(matches!(fx[..], [Effect::LoadHosts]));
    assert_eq!(s.model.selected, None);
}

#[test]
fn import_previews_applies_and_keeps_dialog_on_failure() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    let inv = Inventory {
        hosts: vec![HostEntry::new("web01")],
        global: String::new(),
    };

    update(
        &mut s,
        Msg::ImportOpened(Ok(("empty".into(), Inventory::default()))),
        t0,
    );
    assert!(s.model.import.is_none());
    assert!(s.model.message.as_ref().unwrap().0.contains("No hosts"));

    let fx = update(
        &mut s,
        Msg::ImportOpened(Ok(("hosts.json".into(), inv))),
        t0,
    );
    assert!(matches!(
        fx[..],
        [Effect::PreviewImport {
            mode: ImportMode::Merge,
            ..
        }]
    ));
    assert!(
        update(&mut s, Msg::ImportMode(ImportMode::Merge), t0).is_empty(),
        "same mode"
    );
    let fx = update(&mut s, Msg::ImportMode(ImportMode::Replace), t0);
    assert!(matches!(
        fx[..],
        [Effect::PreviewImport {
            mode: ImportMode::Replace,
            ..
        }]
    ));

    update(&mut s, Msg::ImportPreviewed(Err("db".into())), t0);
    assert!(
        update(&mut s, Msg::ImportApply, t0).is_empty(),
        "no apply on a failed preview"
    );
    update(
        &mut s,
        Msg::ImportPreviewed(Ok(ImportSummary::default())),
        t0,
    );
    let fx = update(&mut s, Msg::ImportApply, t0);
    assert!(matches!(
        fx[..],
        [Effect::ApplyImport {
            mode: ImportMode::Replace,
            ..
        }]
    ));

    update(&mut s, Msg::Imported(Err("constraint".into())), t0);
    let dlg = s.model.import.as_ref().unwrap();
    assert!(
        dlg.preview
            .as_ref()
            .unwrap_err()
            .contains("nothing changed")
    );

    let fx = update(&mut s, Msg::Imported(Ok(ImportSummary::default())), t0);
    assert!(matches!(fx[..], [Effect::LoadHosts]));
    assert!(s.model.import.is_none());
}

#[test]
fn import_from_user_ssh_config_reads_it_without_a_dialog() {
    let mut s = State::default();
    let fx = update(&mut s, Msg::ImportSshConfig, Instant::now());
    assert!(matches!(fx[..], [Effect::ReadUserSshConfig]));
    assert!(matches!(
        update(&mut s, Msg::Import, Instant::now())[..],
        [Effect::PickImportFile]
    ));
}

#[test]
fn collection_results_update_host_state() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    loaded(&mut s, vec![host(1, "web01")], t0);

    update(&mut s, Msg::Collected(Update::Started("web01".into())), t0);
    assert!(s.model.hosts[0].loading);
    let done = |result| Update::Done {
        alias: "web01".into(),
        result,
        elapsed: Duration::from_millis(300),
    };
    let t1 = t0 + Duration::from_secs(1);
    update(&mut s, Msg::Collected(done(Ok(Box::default()))), t1);
    let h = &s.model.hosts[0];
    assert!(!h.loading);
    assert_eq!(h.online(), Some(true));
    assert_eq!(h.updated, Some(t1));

    update(&mut s, Msg::Collected(done(Err("timed out".into()))), t1);
    assert_eq!(s.model.hosts[0].online(), Some(false));
    assert_eq!(s.model.offline_count(), 1);
}

#[test]
fn connect_uses_rdp_for_rdp_only_hosts() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    let mut win = host(2, "win01");
    win.access = Access::Rdp;
    loaded(&mut s, vec![host(1, "web01"), win], t0);
    assert!(matches!(
        update(&mut s, Msg::Ssh(0), t0)[..],
        [Effect::OpenSsh(_)]
    ));
    assert!(matches!(
        update(&mut s, Msg::Ssh(1), t0)[..],
        [Effect::OpenRdp(_)]
    ));
    assert!(update(&mut s, Msg::Ssh(9), t0).is_empty());
}

#[test]
fn messages_expire() {
    let t0 = Instant::now();
    let mut s = State::default();
    update(&mut s, Msg::Notice("hello".into()), t0);
    assert_eq!(
        s.model.message_at(t0 + Duration::from_secs(1)),
        Some("hello")
    );
    assert_eq!(s.model.message_at(t0 + MESSAGE_TTL), None);
}

#[test]
fn clone_copies_only_connection_settings() {
    let src = HostEntry {
        id: Some(5),
        hostname: "192.0.2.32".into(),
        user: "admin".into(),
        port: 2222,
        identity_file: Some("~/.ssh/id_ops".into()),
        note: "web".into(),
        ..HostEntry::new("web32")
    };
    let ed = Editor::clone_of(&src);
    assert_eq!(ed.entry.id, None);
    assert_eq!(ed.entry.alias, "");
    assert_eq!(ed.entry.hostname, "");
    assert_eq!(
        (ed.entry.user.as_str(), ed.port.as_str()),
        ("admin", "2222")
    );
    assert_eq!(ed.identity, "~/.ssh/id_ops");
    assert!(ed.title().contains("cloned from web32"));
}

#[test]
fn switching_access_moves_a_default_port_only() {
    let mut ed = Editor::new(HostEntry::new("x"));
    ed.set_access(Access::Rdp);
    assert_eq!(ed.port, "3389");
    ed.port = "2222".into();
    ed.set_access(Access::Ssh);
    assert_eq!(ed.port, "2222");
}

#[test]
fn filter_matches_names_addresses_and_containers() {
    let mut h = HostState::new(host(1, "web01"));
    assert!(matches_filter("", &h));
    assert!(matches_filter("WEB", &h));
    assert!(matches_filter("192.0.2.1", &h));
    assert!(!matches_filter("nginx", &h));
    let mut r = HostReport::default();
    r.docker.containers.push(telepad_core::model::Container {
        name: "nginx-proxy".into(),
        ..Default::default()
    });
    h.report = Some(r);
    assert!(matches_filter("nginx", &h));
}

fn report(cpu: f32, mem_used_pct: f32, temp: Option<f32>, running: usize) -> HostReport {
    use telepad_core::model::{Container, Memory, Sensor, SensorKind};
    let mut r = HostReport {
        cpu_usage_pct: Some(cpu),
        memory: Some(Memory {
            total_kb: 1000,
            available_kb: (1000.0 - mem_used_pct * 10.0) as u64,
            ..Default::default()
        }),
        os_pretty: "Ubuntu 24.04".into(),
        ..Default::default()
    };
    if let Some(t) = temp {
        r.sensors.push(Sensor {
            kind: SensorKind::Temperature,
            chip: "coretemp".into(),
            label: "Package".into(),
            value: t,
        });
    }
    for _ in 0..running {
        r.docker.containers.push(Container {
            state: "running".into(),
            ..Default::default()
        });
    }
    r
}

fn sort_fixture() -> State {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    let mut a = host(10, "web10");
    a.hostname = "192.0.2.10".into();
    a.note = "Web".into();
    let mut b = host(9, "db09");
    b.hostname = "192.0.2.9".into();
    let mut c = host(2, "nas02");
    c.hostname = "nas.example".into();
    c.note = "Archive".into();
    loaded(&mut s, vec![a, b, c], t0);
    s.model.hosts[0].report = Some(report(80.0, 20.0, Some(70.0), 3));
    s.model.hosts[0].updated = Some(t0);
    s.model.hosts[1].report = Some(report(5.0, 90.0, None, 0));
    s.model.hosts[1].updated = Some(t0 + Duration::from_secs(5));
    s.model.hosts[2].error = Some("connection timed out".into());
    s
}

fn order(s: &State) -> Vec<&str> {
    overview_rows(&s.model, &s.drafts.filter, s.drafts.sort)
        .into_iter()
        .map(|i| s.model.hosts[i].entry.alias.as_str())
        .collect()
}

#[test]
fn header_clicks_cycle_natural_reversed_and_inventory_order() {
    let t0 = Instant::now();
    let mut s = sort_fixture();
    assert_eq!(order(&s), ["web10", "db09", "nas02"], "inventory order");

    update(&mut s, Msg::SortBy(SortKey::Host), t0);
    assert_eq!(order(&s), ["db09", "nas02", "web10"]);
    assert_eq!(s.drafts.sort.direction(SortKey::Host), Some(false));
    update(&mut s, Msg::SortBy(SortKey::Host), t0);
    assert_eq!(order(&s), ["web10", "nas02", "db09"]);
    assert_eq!(s.drafts.sort.direction(SortKey::Host), Some(true));
    update(&mut s, Msg::SortBy(SortKey::Host), t0);
    assert_eq!(s.drafts.sort, Sort::default());
    assert_eq!(order(&s), ["web10", "db09", "nas02"]);

    update(&mut s, Msg::SortBy(SortKey::Cpu), t0);
    assert!(s.drafts.sort.descending, "numbers start highest first");
    assert_eq!(s.drafts.sort.direction(SortKey::Host), None);
}

#[test]
fn numeric_columns_sort_by_value_and_missing_values_go_last() {
    let mut s = sort_fixture();
    let by = |s: &mut State, key, descending| {
        s.drafts.sort = Sort {
            key: Some(key),
            descending,
        };
    };
    by(&mut s, SortKey::Cpu, true);
    assert_eq!(order(&s), ["web10", "db09", "nas02"]);
    by(&mut s, SortKey::Cpu, false);
    assert_eq!(
        order(&s),
        ["db09", "web10", "nas02"],
        "no data stays last both ways"
    );
    by(&mut s, SortKey::Memory, true);
    assert_eq!(order(&s), ["db09", "web10", "nas02"]);
    by(&mut s, SortKey::Temp, true);
    assert_eq!(order(&s), ["web10", "db09", "nas02"]);
    by(&mut s, SortKey::Docker, true);
    assert_eq!(order(&s), ["web10", "db09", "nas02"]);
    by(&mut s, SortKey::Updated, true);
    assert_eq!(order(&s), ["db09", "web10", "nas02"], "most recent first");
    by(&mut s, SortKey::Status, false);
    assert_eq!(order(&s)[0], "nas02", "down hosts first");
    by(&mut s, SortKey::Description, false);
    assert_eq!(
        order(&s),
        ["nas02", "web10", "db09"],
        "empty description last"
    );
}

#[test]
fn addresses_sort_numerically_and_filter_applies_before_sort() {
    let mut s = sort_fixture();
    s.drafts.sort = Sort {
        key: Some(SortKey::Address),
        descending: false,
    };
    assert_eq!(
        order(&s),
        ["db09", "web10", "nas02"],
        "192.0.2.9 < 192.0.2.10 < names"
    );
    s.drafts.filter = "192.0.2".into();
    assert_eq!(order(&s), ["db09", "web10"]);
}

#[test]
fn host_list_view_toggles_are_saved_once_per_change() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    assert!(s.drafts.compact_host_list, "compact by default");
    assert!(
        update(&mut s, Msg::SetCompactHostList(true), t0).is_empty(),
        "no change, no save"
    );
    let fx = update(&mut s, Msg::SetCompactHostList(false), t0);
    assert!(
        matches!(&fx[..], [Effect::SaveUiPrefs(p)] if !p.compact_host_list && !p.group_by_status)
    );
    let fx = update(&mut s, Msg::SetGroupByStatus(true), t0);
    assert!(
        matches!(&fx[..], [Effect::SaveUiPrefs(p)] if !p.compact_host_list && p.group_by_status)
    );
    update(&mut s, Msg::UiPrefsSaved(Err("read-only".into())), t0);
    assert!(s.model.message.as_ref().unwrap().0.contains("read-only"));
}

#[test]
fn host_list_groups_by_status_down_first_and_keeps_filter() {
    let mut s = sort_fixture();
    // web10 and db09 up (reports), nas02 down (error), plus one not checked yet
    s.model.hosts.push(HostState::new(host(4, "new04")));
    let all = host_list_groups(&s.model, "", false);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, None);
    assert_eq!(all[0].1.len(), 4);

    let g = host_list_groups(&s.model, "", true);
    let names: Vec<(&str, Vec<&str>)> = g
        .iter()
        .map(|(grp, rows)| {
            (
                grp.unwrap().title(),
                rows.iter()
                    .map(|&i| s.model.hosts[i].entry.alias.as_str())
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        names,
        [
            ("Down", vec!["nas02"]),
            ("Up", vec!["web10", "db09"]),
            ("Not checked yet", vec!["new04"]),
        ]
    );
    let filtered = host_list_groups(&s.model, "web", true);
    assert_eq!(filtered.len(), 1, "empty groups are left out");
    assert_eq!(filtered[0].0, Some(HostGroup::Up));
}

#[test]
fn worst_usage_picks_the_highest_of_cpu_ram_and_disk() {
    let r = report(30.0, 90.0, None, 0);
    assert_eq!(worst_usage(&r), Some(("RAM", 90.0)));
    let r = report(95.0, 10.0, None, 0);
    assert_eq!(worst_usage(&r).map(|w| w.0), Some("CPU"));
    assert_eq!(worst_usage(&HostReport::default()), None);
}

#[test]
fn overview_card_view_is_saved_and_sort_can_be_picked() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    assert!(!s.drafts.overview_cards, "table by default");
    let fx = update(&mut s, Msg::SetOverviewCards(true), t0);
    assert!(matches!(&fx[..], [Effect::SaveUiPrefs(p)] if p.overview_cards && p.compact_host_list));
    assert!(update(&mut s, Msg::SetOverviewCards(true), t0).is_empty());

    let pick = Sort {
        key: Some(SortKey::Disk),
        descending: true,
    };
    assert!(
        update(&mut s, Msg::SetSort(pick), t0).is_empty(),
        "view-only, nothing saved"
    );
    assert_eq!(s.drafts.sort, pick);
    assert_eq!(SortKey::ALL.len(), 12);
    assert!(SortKey::ALL.iter().all(|k| !k.title().is_empty()));
}

#[test]
fn editor_offers_keys_from_ssh_dir_and_browse_starts_at_current_key() {
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    let fx = update(&mut s, Msg::AddHost, t0);
    assert!(matches!(fx[..], [Effect::FindKeys]));
    update(
        &mut s,
        Msg::KeysFound(vec!["~/.ssh/id_ed25519".into(), "~/.ssh/work".into()]),
        t0,
    );
    assert_eq!(s.drafts.editor.as_ref().unwrap().key_choices.len(), 2);

    s.drafts.editor.as_mut().unwrap().identity = "~/.ssh/work".into();
    let fx = update(&mut s, Msg::EditorBrowseKey, t0);
    assert!(matches!(&fx[..], [Effect::PickKeyFile(p)] if p == "~/.ssh/work"));

    update(&mut s, Msg::EditorClose, t0);
    update(&mut s, Msg::KeysFound(vec!["x".into()]), t0);
    assert!(
        s.drafts.editor.is_none(),
        "late result without an editor is ignored"
    );
}

#[test]
fn maintenance_flags_are_ordered_by_severity_and_sortable() {
    use telepad_core::model::Maintenance;
    let today = Some("2026-11-17".to_string());
    let eol = Maintenance {
        today: today.clone(),
        reboot_required: Some(true),
        reboot_packages: vec!["linux-base".into()],
        updates: Some(98),
        security_updates: Some(6),
        end_of_support: Some("2021-04-30".into()),
    };
    let f = maintenance_flags(&eol);
    let kinds: Vec<FlagKind> = f.iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        [FlagKind::EndOfSupport, FlagKind::Updates, FlagKind::Reboot]
    );
    assert_eq!(f[0].severity, Severity::Danger);
    assert!(f[0].detail.contains("2021-04-30"));
    assert_eq!(f[1].short, "98 (6 sec)");
    assert!(f[2].detail.contains("linux-base"));

    let soon = Maintenance {
        today: today.clone(),
        end_of_support: Some("2027-01-31".into()),
        updates: Some(3),
        security_updates: Some(0),
        ..Default::default()
    };
    let f = maintenance_flags(&soon);
    assert_eq!(
        (f[0].kind, f[0].severity),
        (FlagKind::EndOfSupport, Severity::Warn)
    );
    assert_eq!((f[1].short.as_str(), f[1].severity), ("3", Severity::Info));

    let fine = Maintenance {
        today,
        reboot_required: Some(false),
        updates: Some(0),
        end_of_support: Some("2030-01-01".into()),
        ..Default::default()
    };
    assert!(maintenance_flags(&fine).is_empty());
    assert!(urgency(&eol) > urgency(&soon) && urgency(&soon) > urgency(&fine));
    assert!(SortKey::Flags.starts_descending());
}

#[test]
fn theme_choice_is_saved_once_per_change() {
    use telepad_core::store::ThemePref;
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    assert_eq!(
        s.drafts.theme,
        ThemePref::System,
        "follow the OS by default"
    );
    assert!(update(&mut s, Msg::SetTheme(ThemePref::System), t0).is_empty());
    let fx = update(&mut s, Msg::SetTheme(ThemePref::Light), t0);
    assert!(matches!(&fx[..], [Effect::SaveUiPrefs(p)] if p.theme == ThemePref::Light));
    assert_eq!(ThemePref::parse("dark"), ThemePref::Dark);
    assert_eq!(ThemePref::parse("bogus"), ThemePref::System);
}

#[test]
fn docker_actions_open_terminals_and_portainer_stacks_are_refused() {
    use telepad_core::docker_actions::{ContainerAction, RunInfo, StackAction};
    use telepad_core::model::{Container, HostReport};
    let t0 = Instant::now();
    let (mut s, _) = started(t0);
    let id = |n: u8| format!("{:064x}", n);
    let compose = |name: &str, n: u8| Container {
        id: id(n),
        name: name.into(),
        state: "running".into(),
        stack: "shop".into(),
        working_dir: "/srv/shop".into(),
        config_files: "/srv/shop/compose.yml".into(),
        ..Default::default()
    };
    let portainer = Container {
        id: id(3),
        name: "erp-web".into(),
        state: "running".into(),
        stack: "erp".into(),
        working_dir: "/data/compose/12".into(),
        config_files: "/data/compose/12/docker-compose.yml".into(),
        ..Default::default()
    };
    let mut r = HostReport::default();
    r.docker.containers = vec![compose("shop-web", 1), compose("shop-db", 2), portainer];
    let mut h = HostState::new(host(1, "docker01"));
    h.report = Some(r);
    s.model.hosts = vec![h];

    let fx = update(
        &mut s,
        Msg::DockerAction {
            host: 0,
            container: id(2),
            action: ContainerAction::Restart,
        },
        t0,
    );
    assert!(
        matches!(&fx[..], [Effect::OpenDockerAction(h, p)] if h.alias == "docker01" && p.display() == format!("docker restart {}", id(2)))
    );

    let fx = update(
        &mut s,
        Msg::StackAction {
            host: 0,
            stack: "shop".into(),
            action: StackAction::Up,
        },
        t0,
    );
    assert!(
        matches!(&fx[..], [Effect::OpenDockerAction(_, p)] if p.display().ends_with("-f /srv/shop/compose.yml up -d"))
    );

    let fx = update(
        &mut s,
        Msg::StackAction {
            host: 0,
            stack: "erp".into(),
            action: StackAction::Down,
        },
        t0,
    );
    assert!(fx.is_empty(), "Portainer stacks are changed in Portainer");
    assert!(s.model.message.as_ref().unwrap().0.contains("Portainer"));

    let fx = update(
        &mut s,
        Msg::ShowRunInfo {
            host: 0,
            container: id(1),
        },
        t0,
    );
    assert!(matches!(&fx[..], [Effect::FetchRunInfo(_, c)] if c.name == "shop-web"));
    let stale = RunInfo {
        container: "other".into(),
        ..Default::default()
    };
    update(&mut s, Msg::RunInfoLoaded(stale), t0);
    assert!(matches!(
        s.drafts.run_info,
        Some(RunInfoView::Loading { .. })
    ));
    let ready = RunInfo {
        container: "shop-web".into(),
        docker_run: "docker run".into(),
        ..Default::default()
    };
    update(&mut s, Msg::RunInfoLoaded(ready.clone()), t0);
    assert_eq!(s.drafts.run_info, Some(RunInfoView::Ready(ready)));
    update(&mut s, Msg::CloseRunInfo, t0);
    assert_eq!(s.drafts.run_info, None);
}
