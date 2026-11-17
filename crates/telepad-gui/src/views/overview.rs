//! Overview table: one row per host. Click a header to sort (again to reverse,
//! a third time for inventory order); drag a header border to resize. Cells
//! clip long text with "…" and show it in full on hover, so no value (an
//! error message, a long OS name) can widen its column.

use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder, TableRow};
use telepad_app::{
    HostState, Model, Msg, Sort, SortKey, fullest_disk, hottest, maintenance_flags, overview_rows,
};
use telepad_core::config::Access;
use telepad_core::model::{HostReport, PortainerRole, human_kb};

use crate::icons::{self, label as il};
use crate::widgets::*;

const ROW_HEIGHT: f32 = 24.0;

/// (title, sort key, initial width, minimum width)
const COLUMNS: [(&str, Option<SortKey>, f32, f32); 13] = [
    ("", Some(SortKey::Status), 18.0, 18.0),
    ("Host", Some(SortKey::Host), 130.0, 60.0),
    ("Description", Some(SortKey::Description), 170.0, 40.0),
    ("Address", Some(SortKey::Address), 115.0, 50.0),
    ("OS", Some(SortKey::Os), 190.0, 60.0),
    ("Flags", Some(SortKey::Flags), 150.0, 40.0),
    ("CPU", Some(SortKey::Cpu), 80.0, 50.0),
    ("Memory", Some(SortKey::Memory), 175.0, 60.0),
    ("Fullest disk", Some(SortKey::Disk), 150.0, 60.0),
    ("Temp", Some(SortKey::Temp), 60.0, 40.0),
    ("Docker", Some(SortKey::Docker), 160.0, 50.0),
    ("Updated", Some(SortKey::Updated), 75.0, 50.0),
    ("", None, 70.0, 70.0),
];

pub fn overview(ui: &mut egui::Ui, m: &Model, filter: &str, sort: Sort, out: &mut Vec<Msg>) {
    let rows = overview_rows(m, filter, sort);
    // The table scrolls vertically itself; this adds horizontal scrolling
    // when the columns are wider than the window.
    egui::ScrollArea::horizontal()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut table = TableBuilder::new(ui)
                .id_salt("overview")
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .auto_shrink([false, false]);
            for (i, (_, _, width, min)) in COLUMNS.iter().enumerate() {
                let col = if i == 0 || i == COLUMNS.len() - 1 {
                    Column::exact(*width)
                } else {
                    Column::initial(*width).at_least(*min).clip(true)
                };
                table = table.column(col);
            }
            table
                .header(ROW_HEIGHT, |mut header| {
                    for (title, key, _, _) in COLUMNS {
                        header.col(|ui| {
                            let Some(key) = key else { return };
                            let arrow = match sort.direction(key) {
                                Some(true) => format!(" {}", icons::CARET_DOWN),
                                Some(false) => format!(" {}", icons::CARET_UP),
                                None => String::new(),
                            };
                            let text = if title.is_empty() {
                                format!("{}{arrow}", icons::DOT)
                            } else {
                                format!("{title}{arrow}")
                            };
                            let name = if title.is_empty() { "status" } else { title };
                            if ui
                                .add(egui::Button::new(RichText::new(text).strong()).frame(false))
                                .on_hover_text(format!(
                                    "Sort by {name} (click again to reverse, a third time for inventory order)"
                                ))
                                .clicked()
                            {
                                out.push(Msg::SortBy(key));
                            }
                        });
                    }
                })
                .body(|body| {
                    body.rows(ROW_HEIGHT, rows.len(), |mut row| {
                        let i = rows[row.index()];
                        host_row(&mut row, i, &m.hosts[i], out);
                    });
                });
        });
}

fn host_row(row: &mut TableRow<'_, '_>, i: usize, h: &HostState, out: &mut Vec<Msg>) {
    row.col(|ui| status_dot(ui, h));
    row.col(|ui| {
        if ui
            .add(egui::Link::new(RichText::new(&h.entry.alias).strong()))
            .clicked()
        {
            out.push(Msg::Select(Some(i)));
        }
    });
    row.col(|ui| clipped(ui, RichText::new(&h.entry.note).italics()));
    row.col(|ui| clipped(ui, RichText::new(&h.entry.hostname)));
    match &h.report {
        Some(r) => report_cells(row, h, r),
        None => {
            row.col(|ui| {
                let msg = if let Some(e) = &h.error {
                    RichText::new(e).color(usage_color(95.0))
                } else if h.loading {
                    RichText::new("loading…").weak()
                } else {
                    RichText::new("queued").weak()
                };
                clipped(ui, msg);
            });
            for _ in 0..6 {
                row.col(|_| {});
            }
        }
    }
    row.col(|ui| {
        ui.label(RichText::new(ago(h.updated)).weak());
    });
    row.col(|ui| {
        let (label, tip) = match h.entry.access {
            Access::Ssh => ("SSH", "Open an SSH session in a terminal"),
            Access::Rdp => ("RDP", "Open Remote Desktop"),
        };
        if ui
            .small_button(il(connect_icon(&h.entry), label))
            .on_hover_text(tip)
            .clicked()
        {
            out.push(Msg::Ssh(i));
        }
    });
}

fn report_cells(row: &mut TableRow<'_, '_>, h: &HostState, r: &HostReport) {
    row.col(|ui| {
        icons::os_logo(ui, host_os(h), 16.0);
        clipped(ui, RichText::new(&r.os_pretty));
    });
    row.col(|ui| flag_chips(ui, &maintenance_flags(&r.maintenance)));
    row.col(|ui| match r.cpu_usage_pct {
        Some(c) => bar(ui, c, format!("{c:.0}%"), ui.available_width()),
        None => dash(ui),
    });
    row.col(|ui| match &r.memory {
        Some(mem) => {
            let text = format!("{} / {}", human_kb(mem.used_kb()), human_kb(mem.total_kb));
            bar(ui, mem.used_pct(), text, ui.available_width());
        }
        None => dash(ui),
    });
    row.col(|ui| match fullest_disk(r) {
        Some(d) => {
            let text = format!("{} {:.0}%", d.mount, d.used_pct());
            bar(ui, d.used_pct(), text, ui.available_width());
        }
        None => dash(ui),
    });
    row.col(|ui| {
        match hottest(r) {
            Some(t) => ui.colored_label(temp_color(t), format!("{t:.0}°C")),
            None => ui.label(RichText::new("n/a").weak()),
        };
    });
    row.col(|ui| match docker_text(r) {
        Some(t) => clipped(ui, RichText::new(t)),
        None => dash(ui),
    });
}

/// "12/14 running · agent" (with "(podman)"), or None when Docker is not available.
pub fn docker_text(r: &HostReport) -> Option<String> {
    if r.docker.unavailable.is_some() {
        return None;
    }
    let run = r.docker.containers.iter().filter(|c| c.running()).count();
    let role = match r.docker.portainer().first() {
        Some((PortainerRole::Server, _)) => " · Portainer",
        Some((PortainerRole::Agent, _)) => " · agent",
        None => "",
    };
    let rt = if r.docker.is_podman() {
        " (podman)"
    } else {
        ""
    };
    Some(format!(
        "{run}/{} running{rt}{role}",
        r.docker.containers.len()
    ))
}

/// One line, cut with "…" at the cell edge; the full text shows on hover.
fn clipped(ui: &mut egui::Ui, text: RichText) {
    ui.add(egui::Label::new(text).truncate());
}

fn dash(ui: &mut egui::Ui) {
    ui.label(RichText::new("—").weak());
}
