//! Views. Each one reads the [`Model`], may edit [`Drafts`] (form text,
//! filter, toggles) and pushes [`Msg`]s; none of them does I/O or changes the
//! model directly.

mod cards;
mod confirm;
mod detail;
mod docker;
mod editor;
mod import;
mod overview;
mod panels;
mod ports;
mod sensors;
mod update_window;

use std::time::Instant;

use eframe::egui;
use telepad_app::{Drafts, Model, Msg};

use crate::icons::{self, label as il};
use crate::widgets::usage_color;

/// Draw one frame and return what the user asked for.
pub fn draw(root: &mut egui::Ui, m: &Model, d: &mut Drafts, now: Instant) -> Vec<Msg> {
    let ctx = root.ctx().clone();
    let mut out = Vec::new();

    egui::Panel::top("top").show(root, |ui| panels::top_bar(ui, m, d, &mut out));
    egui::Panel::bottom("status").show(root, |ui| panels::status_bar(ui, m, now));

    if let Some(f) = &m.fatal {
        egui::CentralPanel::default().show(root, |ui| {
            ui.colored_label(usage_color(95.0), f);
        });
        return out;
    }

    egui::Panel::left("hosts")
        .resizable(true)
        .default_size(290.0)
        .show(root, |ui| panels::host_list(ui, m, d, &mut out));

    egui::CentralPanel::default().show(root, |ui| match m.selected {
        None => {
            overview_toolbar(ui, d, &mut out);
            if d.overview_cards {
                cards::cards(ui, m, &d.filter, d.sort, &mut out);
            } else {
                overview::overview(ui, m, &d.filter, d.sort, &mut out);
            }
        }
        Some(i) => {
            if let Some(h) = m.hosts.get(i) {
                detail::detail(ui, h, i, &mut d.ports_nginx_only, &mut out);
            }
        }
    });

    editor::editor_window(&ctx, d, &mut out);
    import::import_window(&ctx, m, &mut out);
    confirm::delete_confirm(&ctx, m, &mut out);
    update_window::update_window(&ctx, m, d, &mut out);
    out
}

/// Table | Cards switch; in card view also "Sort by" (the table sorts by its headers).
fn overview_toolbar(ui: &mut egui::Ui, d: &Drafts, out: &mut Vec<Msg>) {
    use telepad_app::{Sort, SortKey};
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("View").weak());
        if ui
            .selectable_label(!d.overview_cards, il(icons::OVERVIEW, "Table"))
            .clicked()
        {
            out.push(Msg::SetOverviewCards(false));
        }
        if ui
            .selectable_label(d.overview_cards, il(icons::GAUGE, "Cards"))
            .clicked()
        {
            out.push(Msg::SetOverviewCards(true));
        }
        if !d.overview_cards {
            return;
        }
        ui.separator();
        ui.label(egui::RichText::new("Sort by").weak());
        let current = d.sort.key.map_or("Inventory order", SortKey::title);
        egui::ComboBox::from_id_salt("card-sort")
            .selected_text(current)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(d.sort.key.is_none(), "Inventory order")
                    .clicked()
                {
                    out.push(Msg::SetSort(Sort::default()));
                }
                for key in SortKey::ALL {
                    if ui
                        .selectable_label(d.sort.key == Some(key), key.title())
                        .clicked()
                    {
                        out.push(Msg::SetSort(Sort {
                            key: Some(key),
                            descending: key.starts_descending(),
                        }));
                    }
                }
            });
        if let Some(key) = d.sort.key {
            let (icon, tip) = if d.sort.descending {
                (icons::CARET_DOWN, "Highest / Z first — click to reverse")
            } else {
                (icons::CARET_UP, "Lowest / A first — click to reverse")
            };
            if ui.button(icon).on_hover_text(tip).clicked() {
                out.push(Msg::SetSort(Sort {
                    key: Some(key),
                    descending: !d.sort.descending,
                }));
            }
        }
    });
    ui.add_space(4.0);
}
