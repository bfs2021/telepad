//! Import preview dialog (Merge / Replace).

use eframe::egui::{self, RichText};
use telepad_app::{Drafts, Model, Msg};
use telepad_core::store::ImportMode;

use crate::icons::{self, label as il};
use crate::widgets::usage_color;

pub fn import_window(ctx: &egui::Context, m: &Model, out: &mut Vec<Msg>) {
    let Some(d) = &m.import else { return };
    let mut open = true;
    egui::Window::new("Import hosts")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(RichText::new(d.path.display().to_string()).monospace());
            ui.label(format!("{} hosts", d.inventory.hosts.len()));
            if !d.skipped.is_empty() {
                ui.colored_label(
                    usage_color(80.0),
                    format!("{} line(s) / host(s) left out:", d.skipped.len()),
                );
                for s in &d.skipped {
                    ui.label(RichText::new(s).small().monospace());
                }
            }
            if !d.inventory.global.is_empty() {
                ui.label(RichText::new("Includes global options (Host * / Match)").weak());
            }
            ui.add_space(6.0);
            for (mode, text) in [
                (
                    ImportMode::Merge,
                    "Merge — add new, update same alias, keep others",
                ),
                (
                    ImportMode::Replace,
                    "Replace — hosts become exactly this list",
                ),
            ] {
                if ui.radio(d.mode == mode, text).clicked() {
                    out.push(Msg::ImportMode(mode));
                }
            }
            ui.add_space(6.0);
            match &d.preview {
                Ok(p) => {
                    ui.label(format!(
                        "{} added · {} updated · {} unchanged",
                        p.added, p.updated, p.unchanged
                    ));
                    if p.removed > 0 {
                        ui.colored_label(
                            usage_color(95.0),
                            format!("{} host(s) not in the file will be deleted", p.removed),
                        );
                    }
                }
                Err(e) => {
                    ui.colored_label(usage_color(95.0), e);
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        d.preview.is_ok(),
                        egui::Button::new(il(icons::IMPORT, "Import")),
                    )
                    .clicked()
                {
                    out.push(Msg::ImportApply);
                }
                if ui.button("Cancel").clicked() {
                    out.push(Msg::ImportCancel);
                }
            });
        });
    if !open {
        out.push(Msg::ImportCancel);
    }
}

/// "Import from pasted text…": a text box; Next goes to the import preview.
pub fn paste_window(ctx: &egui::Context, d: &mut Drafts, out: &mut Vec<Msg>) {
    let Some(p) = &mut d.paste else { return };
    let mut open = true;
    egui::Window::new("Import from pasted text")
        .collapsible(false)
        .default_width(560.0)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(
                "Paste OpenSSH config text (Host blocks), or one host per line: \
                 [alias] [user@]host[:port]",
            );
            ui.label(
                RichText::new("web01 admin@192.0.2.10\nnas01 192.0.2.20:2222\n198.51.100.7")
                    .monospace()
                    .weak(),
            );
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .max_height(280.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut p.text)
                            .code_editor()
                            .desired_rows(10)
                            .desired_width(f32::INFINITY)
                            .hint_text("Paste here (Ctrl+V)"),
                    );
                });
            for problem in &p.problems {
                ui.colored_label(usage_color(95.0), problem);
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !p.text.trim().is_empty(),
                        egui::Button::new(il(icons::IMPORT, "Next: preview")),
                    )
                    .clicked()
                {
                    out.push(Msg::PasteApply);
                }
                if ui.button(il(icons::COPY, "Paste from clipboard")).clicked() {
                    out.push(Msg::PasteFromClipboard);
                }
                if ui.button("Cancel").clicked() {
                    out.push(Msg::PasteCancel);
                }
            });
        });
    if !open {
        out.push(Msg::PasteCancel);
    }
}
