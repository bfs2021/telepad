//! File windows (#62): WinSCP-style, this computer on the left and the host
//! on the right, transfer queue at the bottom. One native window per host.

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column, TableBuilder};
use telepad_app::files::{
    FileDraft, FileSession, FmMsg, Mode, Pane, Side, SortCol, Status, TState, other,
};
use telepad_app::{Drafts, Model, Msg};
use telepad_core::db::human_size;
use telepad_core::files::transfer::{Decision, Direction};
use telepad_core::files::{Entry, Kind, perm_text, time_text};

use crate::icons::{self, label as il};
use crate::widgets::usage_color;

const ROW: f32 = 22.0;

fn send(out: &mut Vec<Msg>, m: FmMsg) {
    out.push(Msg::Files(m));
}

fn red() -> Color32 {
    usage_color(95.0)
}

fn green() -> Color32 {
    usage_color(10.0)
}

/// Files dragged from one pane.
struct DragFiles {
    window: u64,
    from: Side,
    names: Vec<String>,
}

fn window_icon() -> Option<Arc<egui::IconData>> {
    static ICON: OnceLock<Option<Arc<egui::IconData>>> = OnceLock::new();
    ICON.get_or_init(|| {
        eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon/telepad.png"))
            .ok()
            .map(Arc::new)
    })
    .clone()
}

pub fn file_windows(
    ctx: &egui::Context,
    m: &Model,
    d: &mut Drafts,
    out: &mut Vec<Msg>,
    now: Instant,
) {
    for s in &m.files {
        let Some(dr) = d.files.get_mut(&s.id) else {
            continue;
        };
        let mut builder = egui::ViewportBuilder::default()
            .with_title(format!("Files — {} — Telepad", s.host.alias))
            .with_inner_size([1250.0, 760.0])
            .with_min_inner_size([760.0, 420.0]);
        if let Some(icon) = window_icon() {
            builder = builder.with_icon(icon);
        }
        let vid = egui::ViewportId::from_hash_of(("telepad-files", s.id));
        ctx.show_viewport_immediate(vid, builder, |ui, _| {
            if ui.input(|i| i.viewport().close_requested()) {
                send(out, FmMsg::Close(s.id));
            }
            window(ui, s, dr, out, now);
            ask(ui, s, dr, out);
        });
    }
}

fn window(ui: &mut egui::Ui, s: &FileSession, d: &mut FileDraft, out: &mut Vec<Msg>, now: Instant) {
    let id = s.id;
    egui::Panel::top(("files-top", id)).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(il(icons::FOLDER, &format!("Files — {}", s.host.alias))).strong(),
            );
            ui.label(RichText::new(s.host.target()).monospace().weak());
            match (s.status, s.mode) {
                // A failed connect shows its error and Retry below, not a spinner.
                (Status::Connecting, _) if s.error.is_some() => {}
                (Status::Connecting, _) => {
                    ui.spinner();
                    ui.label("connecting…");
                }
                (Status::Ready, Mode::Sftp) => {
                    ui.label(RichText::new("SFTP").weak());
                }
                (Status::Ready, Mode::Fallback) => {
                    ui.label(
                        RichText::new(format!(
                            "{} no SFTP on the host: copies with scp",
                            icons::WARNING
                        ))
                        .color(usage_color(80.0)),
                    );
                }
            }
            ui.separator();
            ui.checkbox(&mut d.show_hidden, "Hidden files");
            match &s.rsync {
                Some(Ok(())) => {
                    ui.checkbox(&mut d.use_rsync, "Use rsync").on_hover_text(
                        "Copies with rsync over this host's SSH login; shows a dry run first",
                    );
                }
                Some(Err(why)) => {
                    ui.label(RichText::new(format!("rsync: {why}")).weak().small());
                }
                None => {}
            }
        });
    });
    egui::Panel::bottom(("files-queue", id))
        .resizable(true)
        .default_size(190.0)
        .show(ui, |ui| queue(ui, s, d, out, now));
    egui::CentralPanel::default().show(ui, |ui| {
        if let Some(e) = &s.error {
            ui.colored_label(red(), format!("{} {e}", icons::X_CIRCLE));
            if ui.button(il(icons::REFRESH, "Retry")).clicked() {
                send(out, FmMsg::Retry(id));
            }
            return;
        }
        if s.status != Status::Ready {
            ui.spinner();
            return;
        }
        if ui.input(|i| i.key_pressed(egui::Key::F5))
            && let Some(side) = d.active
        {
            let names: Vec<String> = d.selected(side).iter().cloned().collect();
            send(
                out,
                FmMsg::Copy {
                    id,
                    from: side,
                    names,
                },
            );
        }
        ui.columns(2, |cols| {
            pane(&mut cols[0], s, d, Side::Local, out);
            pane(&mut cols[1], s, d, Side::Remote, out);
        });
    });
}

fn pane(ui: &mut egui::Ui, s: &FileSession, d: &mut FileDraft, side: Side, out: &mut Vec<Msg>) {
    let id = s.id;
    let p: &Pane = s.pane(side);
    ui.horizontal(|ui| {
        let (icon, title) = match side {
            Side::Local => (icons::DESKTOP, "This computer".to_string()),
            Side::Remote => (icons::DISKS, s.host.alias.clone()),
        };
        ui.label(RichText::new(il(icon, &title)).strong());
        if p.loading {
            ui.spinner();
        }
        let n = d.selected(side).len();
        if n > 0 {
            ui.label(RichText::new(format!("{n} selected")).weak());
        }
    });
    ui.horizontal(|ui| {
        if ui
            .small_button(icons::CARET_UP)
            .on_hover_text("Up")
            .clicked()
        {
            send(out, FmMsg::Up { id, side });
        }
        if ui
            .small_button(icons::OVERVIEW)
            .on_hover_text("Home")
            .clicked()
        {
            send(out, FmMsg::Home { id, side });
        }
        if ui
            .small_button(icons::REFRESH)
            .on_hover_text("Refresh")
            .clicked()
        {
            send(out, FmMsg::Refresh { id, side });
        }
        let path = match side {
            Side::Local => &mut d.local_path,
            Side::Remote => &mut d.remote_path,
        };
        let w = (ui.available_width() - 150.0).max(120.0);
        let r = ui.add(
            egui::TextEdit::singleline(path)
                .desired_width(w)
                .font(egui::TextStyle::Monospace),
        );
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            send(
                out,
                FmMsg::Navigate {
                    id,
                    side,
                    path: path.clone(),
                },
            );
        }
        let filter = match side {
            Side::Local => &mut d.local_filter,
            Side::Remote => &mut d.remote_filter,
        };
        ui.add(
            egui::TextEdit::singleline(filter)
                .hint_text(format!("{} filter", icons::SEARCH))
                .desired_width(130.0),
        );
    });
    if let Some(e) = &p.error {
        ui.colored_label(red(), format!("{} {e}", icons::X_CIRCLE));
    }
    let filter = match side {
        Side::Local => d.local_filter.clone(),
        Side::Remote => d.remote_filter.clone(),
    };
    let rows: Vec<Entry> = p
        .sorted(d.show_hidden, &filter)
        .into_iter()
        .cloned()
        .collect();
    let has_up = p.path != "/"
        && !(side == Side::Local && telepad_core::files::local_parent(&p.path).is_none());
    // Own fill: the drop zone only highlights while something is dragged.
    let frame = egui::Frame::new()
        .inner_margin(2.0)
        .fill(ui.visuals().extreme_bg_color);
    let (_, dropped) = ui.dnd_drop_zone::<DragFiles, ()>(frame, |ui| {
        table(ui, s, d, side, p, &rows, has_up, out);
    });
    if let Some(drag) = dropped
        && drag.window == id
        && drag.from == other(side)
    {
        send(
            out,
            FmMsg::Copy {
                id,
                from: drag.from,
                names: drag.names.clone(),
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn table(
    ui: &mut egui::Ui,
    s: &FileSession,
    d: &mut FileDraft,
    side: Side,
    p: &Pane,
    rows: &[Entry],
    has_up: bool,
    out: &mut Vec<Msg>,
) {
    let id = s.id;
    let remote = side == Side::Remote;
    let n = rows.len() + usize::from(has_up);
    let height = ui.available_height().max(120.0);
    let mut tb = TableBuilder::new(ui)
        .id_salt(("files-table", id, side))
        .striped(true)
        .resizable(true)
        .auto_shrink([false, false])
        .max_scroll_height(height)
        .sense(egui::Sense::click())
        .column(Column::initial(190.0).at_least(100.0).clip(true))
        .column(Column::initial(70.0).clip(true))
        .column(Column::initial(108.0).clip(true))
        .column(Column::initial(76.0).clip(true));
    if remote {
        tb = tb.column(Column::remainder().at_least(60.0).clip(true));
    }
    let arrow = |col: SortCol| {
        if p.sort == col {
            if p.descending {
                format!(" {}", icons::CARET_DOWN)
            } else {
                format!(" {}", icons::CARET_UP)
            }
        } else {
            String::new()
        }
    };
    let mut cols = vec![
        (SortCol::Name, "Name"),
        (SortCol::Size, "Size"),
        (SortCol::Modified, "Modified (UTC)"),
        (SortCol::Permissions, "Permissions"),
    ];
    if remote {
        cols.push((SortCol::Owner, "Owner"));
    }
    tb.header(ROW, |mut h| {
        for (col, title) in &cols {
            h.col(|ui| {
                if ui
                    .add(
                        egui::Label::new(RichText::new(format!("{title}{}", arrow(*col))).strong())
                            .sense(egui::Sense::click()),
                    )
                    .clicked()
                {
                    send(
                        out,
                        FmMsg::Sort {
                            id,
                            side,
                            col: *col,
                        },
                    );
                }
            });
        }
    })
    .body(|body| {
        body.rows(ROW, n, |mut row| {
            let i = row.index();
            if has_up && i == 0 {
                row.col(|ui| {
                    ui.label(format!("{} ..", icons::FOLDER));
                });
                for _ in 1..cols.len() {
                    row.col(|_| {});
                }
                if row.response().double_clicked() {
                    send(out, FmMsg::Up { id, side });
                }
                return;
            }
            let e = &rows[i - usize::from(has_up)];
            let selected = d.selected(side).contains(&e.name);
            row.set_selected(selected);
            let mut name_resp = None;
            row.col(|ui| {
                let icon = match (e.is_dir(), e.kind) {
                    (true, _) => icons::FOLDER,
                    (false, Kind::Link) => icons::ARROW_RIGHT,
                    _ => icons::INFO,
                };
                let names: Vec<String> = if selected {
                    d.selected(side).iter().cloned().collect()
                } else {
                    vec![e.name.clone()]
                };
                let mut t = RichText::new(format!("{icon} {}", e.name));
                if e.hidden {
                    t = t.weak();
                }
                // One widget for click (select), double-click (open) and drag (copy).
                let r = ui.add(
                    egui::Label::new(t)
                        .truncate()
                        .selectable(false)
                        .sense(egui::Sense::click_and_drag()),
                );
                if r.drag_started() {
                    egui::DragAndDrop::set_payload(
                        ui.ctx(),
                        DragFiles {
                            window: id,
                            from: side,
                            names,
                        },
                    );
                }
                if r.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                }
                name_resp = Some(r);
            });
            row.col(|ui| {
                if !e.is_dir() {
                    ui.label(human_size(e.size));
                }
            });
            row.col(|ui| {
                ui.label(RichText::new(time_text(e.mtime)).monospace().small());
            });
            row.col(|ui| {
                ui.label(RichText::new(perm_text(e.perm)).monospace().small());
            });
            if remote {
                row.col(|ui| {
                    ui.label(&e.owner);
                });
            }
            let resp = row.response();
            let name_resp = name_resp.map(|r| r | resp.clone()).unwrap_or(resp);
            if name_resp.double_clicked() {
                if e.is_dir() {
                    send(
                        out,
                        FmMsg::Enter {
                            id,
                            side,
                            name: e.name.clone(),
                        },
                    );
                }
            } else if name_resp.clicked() {
                d.active = Some(side);
                let ctrl = name_resp
                    .ctx
                    .input(|i| i.modifiers.command || i.modifiers.ctrl);
                let sel = d.selected_mut(side);
                if ctrl {
                    if !sel.remove(&e.name) {
                        sel.insert(e.name.clone());
                    }
                } else {
                    sel.clear();
                    sel.insert(e.name.clone());
                }
            }
        });
    });
}

fn queue(ui: &mut egui::Ui, s: &FileSession, d: &mut FileDraft, out: &mut Vec<Msg>, now: Instant) {
    let id = s.id;
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let ready = s.status == Status::Ready;
        let up: Vec<String> = d.selected_local.iter().cloned().collect();
        let down: Vec<String> = d.selected_remote.iter().cloned().collect();
        if ui
            .add_enabled(
                ready && !up.is_empty(),
                egui::Button::new(il(icons::ARROW_RIGHT, "Upload")),
            )
            .on_hover_text("Copy the selected local files to the host folder (F5, or drag them)")
            .clicked()
        {
            send(
                out,
                FmMsg::Copy {
                    id,
                    from: Side::Local,
                    names: up,
                },
            );
        }
        if ui
            .add_enabled(
                ready && !down.is_empty(),
                egui::Button::new(il(icons::ARROW_LEFT, "Download")),
            )
            .on_hover_text("Copy the selected host files to the local folder (F5, or drag them)")
            .clicked()
        {
            send(
                out,
                FmMsg::Copy {
                    id,
                    from: Side::Remote,
                    names: down,
                },
            );
        }
        ui.separator();
        ui.label(RichText::new("Transfers").strong());
        if s.transfers.iter().any(|t| t.state.finished())
            && ui.small_button("Clear finished").clicked()
        {
            send(out, FmMsg::ClearFinished(id));
        }
    });
    egui::ScrollArea::vertical()
        .id_salt(("files-queue-scroll", id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if s.transfers.is_empty() {
                ui.label(RichText::new("Select files and click Upload / Download, press F5, or drag them to the other side.").weak());
            }
            for t in s.transfers.iter().rev() {
                ui.horizontal_wrapped(|ui| {
                    let icon = match t.direction {
                        Direction::Upload => icons::ARROW_RIGHT,
                        Direction::Download => icons::ARROW_LEFT,
                    };
                    ui.label(RichText::new(format!("{icon} {}", t.title)).monospace())
                        .on_hover_text(&t.dest);
                    if t.rsync {
                        ui.label(RichText::new("rsync").weak().small());
                    }
                    let state = match &t.state {
                        TState::Planning => "listing…".to_string(),
                        TState::Asking => "waiting for your answer".to_string(),
                        TState::Previewed(_) => "dry run done".to_string(),
                        TState::Running => String::new(),
                        TState::Done { copied, skipped, failed } => {
                            format!("done: {copied} copied, {skipped} skipped, {failed} failed")
                        }
                        TState::Failed(e) => format!("failed: {e}"),
                        TState::Cancelled => "cancelled".to_string(),
                    };
                    let color = match &t.state {
                        TState::Failed(_) => red(),
                        TState::Done { failed, .. } if *failed > 0 => red(),
                        TState::Done { .. } => green(),
                        _ => ui.visuals().text_color(),
                    };
                    if !state.is_empty() {
                        ui.colored_label(color, state);
                    }
                    if matches!(t.state, TState::Planning | TState::Running | TState::Asking | TState::Previewed(_))
                        && ui.small_button(il(icons::STOP, "Cancel")).clicked()
                    {
                        send(out, FmMsg::Cancel { id, transfer: t.id });
                    }
                });
                if t.state == TState::Running {
                    let frac = if t.bytes_total > 0 {
                        (t.bytes_done as f32 / t.bytes_total as f32).min(1.0)
                    } else {
                        0.0
                    };
                    let speed = t
                        .speed(now)
                        .map(|b| format!(" · {}/s", human_size(b as u64)))
                        .unwrap_or_default();
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .desired_width(ui.available_width().min(700.0))
                            .text(format!(
                                "{} of {}{speed} · {}",
                                human_size(t.bytes_done),
                                human_size(t.bytes_total),
                                t.current
                            )),
                    );
                }
                if let TState::Previewed(list) = &t.state {
                    ui.label(format!("rsync would change {} path(s):", list.len()));
                    for l in list.iter().take(12) {
                        ui.label(RichText::new(format!("  {l}")).monospace().small());
                    }
                    if list.len() > 12 {
                        ui.label(RichText::new(format!("  … and {} more", list.len() - 12)).weak().small());
                    }
                    if ui.button(il(icons::PLAY, "Run rsync")).clicked() {
                        send(out, FmMsg::RsyncRun { id, transfer: t.id });
                    }
                }
                if !t.errors.is_empty() {
                    egui::CollapsingHeader::new(RichText::new(format!("{} error(s)", t.errors.len())).color(red()))
                        .id_salt(("files-errors", id, t.id))
                        .show(ui, |ui| {
                            for e in &t.errors {
                                ui.label(RichText::new(e).monospace().small());
                            }
                        });
                }
                ui.separator();
            }
        });
}

/// "File exists" question, one file at a time (Apply to all for the rest).
fn ask(ui: &mut egui::Ui, s: &FileSession, d: &mut FileDraft, out: &mut Vec<Msg>) {
    let id = s.id;
    let Some(a) = &s.ask else {
        return;
    };
    let Some(t) = s.transfer(a.transfer) else {
        return;
    };
    let Some(it) = a.conflicts.get(a.at).and_then(|&i| t.items.get(i)) else {
        return;
    };
    let modal = egui::Modal::new(egui::Id::unique(("files-ask", id))).show(ui.ctx(), |ui| {
        ui.set_width(520.0);
        ui.heading(il(icons::WARNING, "The file exists"));
        ui.label(RichText::new(&it.label).monospace().strong());
        let (ssize, smtime) = (it.size, it.mtime);
        let (dsize, dmtime) = it.exists.unwrap_or_default();
        let (from, to) = match t.direction {
            Direction::Upload => ("this computer", s.host.alias.as_str()),
            Direction::Download => (s.host.alias.as_str(), "this computer"),
        };
        egui::Grid::new(("files-ask-grid", id))
            .num_columns(3)
            .spacing([14.0, 4.0])
            .show(ui, |ui| {
                ui.label("");
                ui.strong("Size");
                ui.strong("Modified (UTC)");
                ui.end_row();
                ui.label(format!("New ({from})"));
                ui.label(human_size(ssize));
                ui.label(time_text(smtime));
                ui.end_row();
                ui.label(format!("Existing ({to})"));
                ui.label(human_size(dsize));
                ui.label(time_text(dmtime));
                ui.end_row();
            });
        let left = a.conflicts.len() - a.at;
        ui.checkbox(
            &mut d.apply_all,
            format!("Apply to all {left} existing file(s)"),
        );
        ui.horizontal(|ui| {
            if ui.button(RichText::new("Overwrite").color(red())).clicked() {
                send(
                    out,
                    FmMsg::Decide {
                        id,
                        decision: Decision::Overwrite,
                    },
                );
            }
            if ui.button("Skip").clicked() {
                send(
                    out,
                    FmMsg::Decide {
                        id,
                        decision: Decision::Skip,
                    },
                );
            }
            if ui
                .button("Newer only")
                .on_hover_text("Overwrite only when the new file is newer")
                .clicked()
            {
                send(
                    out,
                    FmMsg::Decide {
                        id,
                        decision: Decision::NewerOnly,
                    },
                );
            }
            if ui.button("Cancel transfer").clicked() {
                send(out, FmMsg::AskCancel(id));
            }
        });
    });
    if modal.should_close() {
        send(out, FmMsg::AskCancel(id));
    }
}
