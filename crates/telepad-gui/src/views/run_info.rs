//! "How it runs" window: reconstructed `docker run` and the compose files,
//! fetched read-only, secrets masked.

use eframe::egui::{self, RichText};
use telepad_app::{Drafts, Msg, RunInfoView};

use crate::icons::{self, label as il};
use crate::widgets::usage_color;

pub fn run_info_window(ctx: &egui::Context, d: &Drafts, out: &mut Vec<Msg>) {
    let Some(view) = &d.run_info else {
        return;
    };
    let title = match view {
        RunInfoView::Loading { container, .. } => format!("How {container} runs"),
        RunInfoView::Ready(info) => format!("How {} runs", info.container),
    };
    let mut open = true;
    egui::Window::new(title)
        .open(&mut open)
        .default_size([760.0, 520.0])
        .resizable(true)
        .show(ctx, |ui| match view {
            RunInfoView::Loading { host, .. } => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Reading docker inspect on {host}…"));
                });
            }
            RunInfoView::Ready(info) => {
                if let Some(e) = &info.error {
                    ui.colored_label(usage_color(95.0), e);
                    return;
                }
                ui.label(
                    RichText::new(
                        "Rebuilt from docker inspect: image defaults (environment, command) show up \
                         as options. Values that look like secrets are replaced by ***.",
                    )
                    .weak(),
                );
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong("docker run");
                        if ui.button(il(icons::COPY, "Copy")).clicked() {
                            ui.ctx().copy_text(info.docker_run.clone());
                        }
                    });
                    code(ui, &info.docker_run);
                    for (path, text) in &info.compose_files {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.strong(path);
                            if ui.button(il(icons::COPY, "Copy")).clicked() {
                                ui.ctx().copy_text(text.clone());
                            }
                        });
                        code(ui, text);
                    }
                });
            }
        });
    if !open {
        out.push(Msg::CloseRunInfo);
    }
}

/// Selectable, read-only monospace text.
fn code(ui: &mut egui::Ui, text: &str) {
    let mut t = text;
    ui.add(
        egui::TextEdit::multiline(&mut t)
            .code_editor()
            .desired_width(f32::INFINITY),
    );
}
