//! Top bar, status bar and the host list on the left.

use std::time::Instant;

use eframe::egui::{self, RichText};
use telepad_app::{
    Drafts, HostState, Model, Msg, UpdatePhase, fullest_disk, host_list_groups, maintenance_flags,
    worst_usage,
};
use telepad_core::update::CURRENT_VERSION;

use crate::icons::{self, label as il};
use crate::widgets::*;

pub fn top_bar(ui: &mut egui::Ui, m: &Model, d: &mut Drafts, out: &mut Vec<Msg>) {
    ui.add_space(4.0);
    // Wraps onto a second line on narrow windows instead of clipping the counts.
    ui.horizontal_wrapped(|ui| {
        ui.heading(il(icons::DATABASE, "Telepad"));
        ui.separator();
        ui.menu_button(il(icons::MENU, "Hosts"), |ui| {
            for (group, items) in [
                ("Add", vec![(icons::PLUS, "Add one host…", Msg::AddHost)]),
                (
                    "Import",
                    vec![
                        (icons::IMPORT, "Import from a file…", Msg::Import),
                        (icons::IMPORT, "Import ~/.ssh/config", Msg::ImportSshConfig),
                    ],
                ),
                ("Export", vec![(icons::EXPORT, "Export…", Msg::Export)]),
            ] {
                ui.label(RichText::new(group).weak().small());
                for (icon, text, msg) in items {
                    if ui.button(il(icon, text)).clicked() {
                        out.push(msg);
                        ui.close();
                    }
                }
                ui.separator();
            }
            ui.separator();
            if ui.button(il(icons::IMPORT, "Software update…")).clicked() {
                out.push(Msg::UpdateWindow(true));
                ui.close();
            }
        });
        match &m.update.phase {
            UpdatePhase::Installed(v) => {
                if ui
                    .button(
                        RichText::new(il(icons::REFRESH, &format!("Restart for {v}")))
                            .color(usage_color(0.0))
                            .strong(),
                    )
                    .clicked()
                {
                    out.push(Msg::Restart);
                }
            }
            UpdatePhase::Installing => {
                ui.spinner();
                ui.label("updating…");
            }
            _ => {
                if let Some(c) = m.update.banner()
                    && ui
                        .button(
                            RichText::new(il(
                                icons::IMPORT,
                                &format!("Update {} available", c.version),
                            ))
                            .color(usage_color(0.0))
                            .strong(),
                        )
                        .on_hover_text(format!("Installed: {CURRENT_VERSION}"))
                        .clicked()
                {
                    out.push(Msg::UpdateWindow(true));
                }
            }
        }
        if ui
            .add_enabled(
                m.busy == 0,
                egui::Button::new(il(icons::REFRESH, "Refresh all")),
            )
            .clicked()
        {
            out.push(Msg::RefreshAll);
        }
        ui.checkbox(&mut d.auto_refresh, "Auto every");
        ui.add(
            egui::DragValue::new(&mut d.interval_secs)
                .range(15..=3600)
                .suffix(" s"),
        );
        ui.separator();
        ui.label(RichText::new(icons::SEARCH).size(16.0));
        ui.add(
            egui::TextEdit::singleline(&mut d.filter)
                .desired_width(180.0)
                .hint_text("host, IP, OS, container"),
        );
        ui.separator();
        ui.label(
            RichText::new(format!("{} {} up", icons::DOT, m.online_count()))
                .color(usage_color(0.0)),
        );
        ui.label(
            RichText::new(format!("{} {} down", icons::DOT, m.offline_count()))
                .color(usage_color(95.0)),
        );
        ui.label(format!("{} hosts", m.hosts.len()));
        if m.busy > 0 {
            ui.spinner();
            ui.label(format!("{} loading", m.busy));
        }
    });
    ui.add_space(4.0);
}

pub fn status_bar(ui: &mut egui::Ui, m: &Model, now: Instant) {
    ui.horizontal(|ui| {
        if let Some(db) = &m.db_path {
            let label = ui.label(RichText::new(format!("database: {}", db.display())).weak());
            if let Some(cfg) = &m.ssh_config_path {
                label.on_hover_text(format!("ssh uses the generated file {}", cfg.display()));
            }
        }
        ui.separator();
        let v = ui.label(RichText::new(format!("v{CURRENT_VERSION}")).weak());
        if let Some(e) = &m.update.error {
            let tip = format!("Update check failed: {e}");
            v.on_hover_text(&tip);
            ui.label(RichText::new(icons::WARNING).weak())
                .on_hover_text(tip);
        }
        if let Some(text) = m.message_at(now) {
            ui.separator();
            ui.label(text);
        }
    });
}

pub fn host_list(ui: &mut egui::Ui, m: &Model, d: &Drafts, out: &mut Vec<Msg>) {
    ui.add_space(4.0);
    let overview = egui::Button::selectable(
        m.selected.is_none(),
        RichText::new(il(icons::OVERVIEW, "Overview — all hosts"))
            .strong()
            .size(17.0),
    )
    .min_size(egui::vec2(ui.available_width(), 38.0));
    if ui
        .add(overview)
        .on_hover_text("All hosts as a table or cards")
        .clicked()
    {
        out.push(Msg::Select(None));
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("View").weak());
        if ui
            .selectable_label(d.compact_host_list, "Compact")
            .on_hover_text("One line per host")
            .clicked()
        {
            out.push(Msg::SetCompactHostList(true));
        }
        if ui
            .selectable_label(!d.compact_host_list, "Detailed")
            .on_hover_text("Description and CPU / RAM / disk bars under each host")
            .clicked()
        {
            out.push(Msg::SetCompactHostList(false));
        }
        let mut group = d.group_by_status;
        if ui
            .checkbox(&mut group, "Group by status")
            .on_hover_text("Down hosts first, then up, then not checked yet")
            .changed()
        {
            out.push(Msg::SetGroupByStatus(group));
        }
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (group, rows) in host_list_groups(m, &d.filter, d.group_by_status) {
                let mut draw = |ui: &mut egui::Ui| {
                    for &i in &rows {
                        let h = &m.hosts[i];
                        if d.compact_host_list {
                            compact_row(ui, m, i, h, out);
                        } else {
                            detailed_row(ui, m, i, h, out);
                        }
                    }
                };
                match group {
                    None => draw(ui),
                    Some(g) => {
                        egui::CollapsingHeader::new(
                            RichText::new(format!("{} ({})", g.title(), rows.len())).strong(),
                        )
                        .id_salt(g.title())
                        .default_open(true)
                        .show(ui, draw);
                    }
                }
            }
        });
}

/// Name row shared by both views; returns the clickable label.
fn name_row(
    ui: &mut egui::Ui,
    m: &Model,
    i: usize,
    h: &HostState,
    extra: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    ui.horizontal(|ui| {
        status_dot(ui, h);
        let mut name = RichText::new(&h.entry.alias).strong();
        if h.entry.unmonitored {
            name = name.weak();
        }
        icons::os_logo(ui, host_os(h), 16.0);
        let label = ui.selectable_label(m.selected == Some(i), name);
        extra(ui);
        label
    })
    .inner
}

fn row_actions(resp: &egui::Response, i: usize, out: &mut Vec<Msg>) {
    if resp.clicked() {
        out.push(Msg::Select(Some(i)));
    }
    resp.context_menu(|ui| {
        for (icon, text, msg) in [
            (icons::EDIT, "Edit", Msg::Edit(i)),
            (
                icons::CLONE,
                "Clone (User, Port, IdentityFile)",
                Msg::Clone(i),
            ),
            (icons::TRASH, "Delete", Msg::Delete(i)),
        ] {
            if ui.button(il(icon, text)).clicked() {
                out.push(msg);
                ui.close();
            }
        }
    });
}

/// One line: status, logo, alias, and on the right the worst of CPU / RAM /
/// disk (or a warning sign for an error). Details on hover.
fn compact_row(ui: &mut egui::Ui, m: &Model, i: usize, h: &HostState, out: &mut Vec<Msg>) {
    let resp = name_row(ui, m, i, h, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(r) = &h.report {
                if let Some((what, pct)) = worst_usage(r) {
                    let mut tip = String::new();
                    if let Some(c) = r.cpu_usage_pct {
                        tip.push_str(&format!("CPU {c:.0}%"));
                    }
                    if let Some(mem) = &r.memory {
                        tip.push_str(&format!("   RAM {:.0}%", mem.used_pct()));
                    }
                    if let Some(disk) = fullest_disk(r) {
                        tip.push_str(&format!("   Disk {:.0}% ({})", disk.used_pct(), disk.mount));
                    }
                    bar(ui, pct, format!("{what} {pct:.0}%"), 86.0);
                    ui.response().on_hover_text(tip.trim().to_string());
                }
                let flags = maintenance_flags(&r.maintenance);
                if let Some(top) = flags.first() {
                    let all: Vec<&str> = flags.iter().map(|f| f.detail.as_str()).collect();
                    ui.label(
                        RichText::new(flag_icon(top.kind)).color(severity_color(top.severity)),
                    )
                    .on_hover_text(all.join("\n"));
                }
            } else if let Some(e) = &h.error {
                ui.label(RichText::new(icons::WARNING).color(usage_color(95.0)))
                    .on_hover_text(e);
            } else if h.loading {
                ui.spinner();
            }
        });
    });
    let hover = match (&h.entry.note, &h.entry.hostname) {
        (n, a) if n.is_empty() => a.clone(),
        (n, a) => format!("{n}\n{a}"),
    };
    let resp = if hover.is_empty() {
        resp
    } else {
        resp.on_hover_text(hover)
    };
    row_actions(&resp, i, out);
}

/// The original three-line row: name + address, description, CPU / RAM / disk bars (or the error).
fn detailed_row(ui: &mut egui::Ui, m: &Model, i: usize, h: &HostState, out: &mut Vec<Msg>) {
    let resp = name_row(ui, m, i, h, |ui| {
        ui.label(RichText::new(&h.entry.hostname).weak().small());
    });
    row_actions(&resp, i, out);
    if !h.entry.note.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new(&h.entry.note).small().italics());
        });
    }
    if let Some(r) = &h.report {
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            if let Some(c) = r.cpu_usage_pct {
                bar(ui, c, format!("CPU {c:.0}%"), 80.0);
            }
            if let Some(mem) = &r.memory {
                let p = mem.used_pct();
                bar(ui, p, format!("RAM {p:.0}%"), 80.0);
            }
            if let Some(disk) = fullest_disk(r) {
                let p = disk.used_pct();
                bar(ui, p, format!("Disk {p:.0}%"), 80.0);
            }
        });
    } else if let Some(e) = &h.error {
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new(e).small().color(usage_color(95.0)));
        });
    }
    ui.add_space(2.0);
}
