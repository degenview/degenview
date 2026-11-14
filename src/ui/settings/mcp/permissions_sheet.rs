//! The sheet that edits a client's permissions, or the maximum: presets, permission groups,
//! remote ranges and the footer.

use super::controls::{caption, field};
use super::{CONTROL_HEIGHT, Editing, McpSettingsInput, McpSettingsState, SHEET_WIDTH, initial};
use crate::model::mcp::{Cidr, MAX_CIDRS, McpAction, McpClient, Permission, PermissionSet};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{Tone, time_text::relative_time};
use crate::ui::{
    kit::{self, sheet},
    settings::card,
    theme,
};
use eframe::egui::{self, Align, Id, Layout, Margin, Rect, Sense, Ui, Vec2, pos2, vec2};

pub(super) fn permissions_sheet(
    ctx: &egui::Context,
    state: &mut McpSettingsState,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    let Some(editing) = state.sheet.clone() else {
        return;
    };
    // The client was revoked (here or elsewhere): nothing left to edit.
    let client = match &editing {
        Editing::Client(id) => match input.clients.iter().find(|client| client.id == *id) {
            Some(client) => Some(client),
            None => {
                state.sheet = None;
                state.confirm_revoke = false;
                return;
            }
        },
        Editing::Maximum | Editing::RemoteCeiling => None,
    };
    let set = match (&editing, client) {
        (Editing::RemoteCeiling, _) => &input.settings.remote_ceiling,
        (_, Some(client)) => &client.permissions,
        (_, None) => input.global,
    };
    let mut close = false;

    sheet::show(ctx, Id::new("mcp-permissions"), SHEET_WIDTH, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| match client {
            Some(client) => {
                let used = client.last_used_ms.map_or_else(
                    || "never used".to_owned(),
                    |used| format!("last used {}", relative_time(input.now_ms, used)),
                );
                sheet::header_with(
                    ui,
                    &client.name,
                    &format!(
                        "Created {} · {used}",
                        relative_time(input.now_ms, client.created_at_ms)
                    ),
                    |ui| {
                        let (badge, _) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::hover());
                        ui.painter()
                            .rect_filled(badge, 12.0, theme::with_alpha(kit::ACCENT, 0.14));
                        ui.painter().text(
                            badge.center(),
                            egui::Align2::CENTER_CENTER,
                            initial(&client.name),
                            egui::FontId::proportional(20.0),
                            kit::ACCENT,
                        );
                    },
                );
            }
            None if editing == Editing::RemoteCeiling => sheet::header(
                ui,
                Icon::Link,
                "Remote ceiling",
                "The most a credential with remote access can do, even on this Mac.",
                theme::text_secondary(),
            ),
            None => sheet::header(
                ui,
                Icon::Sliders,
                "Maximum permissions",
                "The most any credential can do. Each one can only narrow this.",
                kit::ACCENT,
            ),
        });
        // A client's sheet carries more above the groups than the other two, so its presets
        // scroll with them rather than leaving the groups a row or two of the window.
        if client.is_none() {
            ui.add_space(16.0);
            sheet::inset(ui, |ui| presets(ui, &editing, set, actions));
        }
        ui.add_space(14.0);

        let list_height = (ctx.content_rect().height() - 330.0).clamp(160.0, 420.0);
        egui::ScrollArea::vertical()
            .max_height(list_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                sheet::inset(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    // With the groups, so a short window scrolls them together instead of leaving
                    // the permissions a row or two to show in.
                    if let Some(client) = client {
                        client_remote_block(ui, state, client, actions);
                        ui.add_space(14.0);
                        presets(ui, &editing, set, actions);
                        ui.add_space(16.0);
                    }
                    for (index, group) in Permission::GROUPS.into_iter().enumerate() {
                        if index > 0 {
                            ui.add_space(16.0);
                        }
                        group_card(ui, group, &editing, set, input.global, actions);
                    }
                    ui.add_space(12.0);
                    caption(ui, "Changes apply immediately.");
                });
            });

        ui.add_space(14.0);
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(14.0);
            sheet_footer(ui, state, client, actions, &mut close);
            ui.add_space(14.0);
        });
    });

    // Escape backs out of the revoke question first, then closes.
    if !egui::Popup::is_any_open(ctx) && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        if state.confirm_revoke {
            state.confirm_revoke = false;
        } else {
            close = true;
        }
    }
    if close {
        state.sheet = None;
        state.confirm_revoke = false;
    }
}

/// "Use from other machines" and, once on, the only addresses it may call from.
fn client_remote_block(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    client: &McpClient,
    actions: &mut Vec<McpAction>,
) {
    kit::card_frame(false)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                ui.set_min_height(CONTROL_HEIGHT);
                ui.vertical(|ui| {
                    ui.set_max_width(ui.available_width() - 56.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(kit::text(
                        "Use from other machines",
                        12.5,
                        kit::MEDIUM,
                        theme::text_primary(),
                    ));
                    caption(
                        ui,
                        if client.remote_allowed {
                            "Held to the remote ceiling, and every change waits for your approval."
                        } else {
                            "Off: this credential works from this Mac only."
                        },
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut on = client.remote_allowed;
                    if card::switch(ui, &mut on) {
                        state.range_error = None;
                        actions.push(McpAction::SetClientRemote {
                            client_id: client.id.clone(),
                            allowed: on,
                        });
                    }
                });
            });
            if !client.remote_allowed {
                return;
            }
            kit::divider(ui);
            ui.label(kit::text(
                "Only from these addresses",
                kit::size::CAPTION + 0.5,
                kit::SEMIBOLD,
                theme::text_secondary(),
            ));
            if client.allowed_cidrs.is_empty() {
                caption(ui, "Any address that has the token.");
            } else {
                let mut removed = None;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for (index, cidr) in client.allowed_cidrs.iter().enumerate() {
                        if range_chip(ui, &cidr.to_string()) {
                            removed = Some(index);
                        }
                    }
                });
                if let Some(index) = removed {
                    let mut cidrs = client.allowed_cidrs.clone();
                    cidrs.remove(index);
                    actions.push(McpAction::SetClientCidrs {
                        client_id: client.id.clone(),
                        cidrs,
                    });
                }
            }
            let full = client.allowed_cidrs.len() >= MAX_CIDRS;
            let parsed = Cidr::parse(&state.range_text);
            let ready = parsed.is_ok() && !full;
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), CONTROL_HEIGHT),
                Layout::right_to_left(Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let mut add = kit::sheet_button(ui, "Add", ready, ready).clicked();
                    let width = ui.available_width();
                    let response = field(
                        ui,
                        Id::new("mcp-range"),
                        &mut state.range_text,
                        "192.168.1.0/24",
                        width,
                    );
                    state.range_text.truncate(50);
                    add |= response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if response.changed() {
                        state.range_error = None;
                    }
                    if add {
                        match &parsed {
                            _ if full => {
                                state.range_error =
                                    Some(format!("At most {MAX_CIDRS} address ranges"));
                            }
                            Ok(cidr) => {
                                let mut cidrs = client.allowed_cidrs.clone();
                                if !cidrs.contains(cidr) {
                                    cidrs.push(*cidr);
                                    actions.push(McpAction::SetClientCidrs {
                                        client_id: client.id.clone(),
                                        cidrs,
                                    });
                                }
                                state.range_text.clear();
                                state.range_error = None;
                            }
                            Err(error) => state.range_error = Some(error.clone()),
                        }
                    }
                },
            );
            if let Some(error) = &state.range_error {
                ui.label(kit::text(
                    error,
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    Tone::Bad.text(),
                ));
            } else {
                caption(
                    ui,
                    "An address or a range, like 192.168.1.20 or 10.0.0.0/8. A request from anywhere else is refused as if the token were wrong.",
                );
            }
        });
}

/// An address range as a capsule with a remove mark. Returns whether the mark was clicked.
fn range_chip(ui: &mut Ui, text: &str) -> bool {
    let galley = kit::galley(
        ui,
        text,
        kit::size::CAPTION + 0.5,
        kit::MEDIUM,
        theme::text_primary(),
        f32::INFINITY,
    );
    let size = vec2(galley.size().x + 14.0 + 20.0, 22.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, 6.0, theme::with_alpha(theme::text_secondary(), 0.14));
    ui.painter().galley(
        pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::text_primary(),
    );
    let mark = Rect::from_center_size(
        pos2(rect.right() - 11.0, rect.center().y),
        Vec2::splat(18.0),
    );
    let response = ui.interact(
        mark,
        ui.id().with(("mcp-range-remove", text)),
        Sense::click(),
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(mark.center(), Vec2::splat(8.0)),
        Icon::Xmark,
        if response.hovered() {
            theme::text_primary()
        } else {
            theme::text_tertiary()
        },
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Remove this range")
        .clicked()
}

/// "Read only" / "Everything" / "None": the whole set in one step.
fn presets(ui: &mut Ui, editing: &Editing, current: &PermissionSet, actions: &mut Vec<McpAction>) {
    let choose = |set: PermissionSet, actions: &mut Vec<McpAction>| {
        if &set != current {
            actions.push(match editing {
                Editing::Maximum => McpAction::SetGlobalPermissions(set),
                Editing::RemoteCeiling => McpAction::SetRemoteCeiling(set),
                Editing::Client(id) => McpAction::SetClientPermissions {
                    client_id: id.clone(),
                    permissions: set,
                },
            });
        }
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(kit::text(
            "Preset",
            kit::size::CAPTION + 0.5,
            kit::SEMIBOLD,
            theme::text_secondary(),
        ));
        if kit::sheet_button(ui, "Read only", false, true).clicked() {
            choose(PermissionSet::read_only(), actions);
        }
        if kit::sheet_button(ui, "Everything", false, true).clicked() {
            choose(PermissionSet::everything(), actions);
        }
        if kit::sheet_button(ui, "None", false, true).clicked() {
            choose(PermissionSet::none(), actions);
        }
    });
}

/// One group's switches in a card, each with what it allows.
fn group_card(
    ui: &mut Ui,
    group: &'static str,
    editing: &Editing,
    set: &PermissionSet,
    cap: &PermissionSet,
    actions: &mut Vec<McpAction>,
) {
    let permissions: Vec<Permission> = Permission::ALL
        .into_iter()
        .filter(|permission| permission.group() == group)
        .collect();
    let allowed = permissions.iter().filter(|p| set.contains(**p)).count();
    ui.horizontal(|ui| {
        ui.label(kit::text(
            group,
            kit::size::CAPTION + 0.5,
            kit::SEMIBOLD,
            theme::text_secondary(),
        ));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(kit::text(
                format!("{allowed} of {}", permissions.len()),
                kit::size::CAPTION,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        });
    });
    ui.add_space(6.0);
    kit::card_frame(false)
        .inner_margin(Margin::symmetric(12, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, permission) in permissions.iter().copied().enumerate() {
                if index > 0 {
                    kit::divider(ui);
                }
                permission_row(ui, permission, editing, set, cap, actions);
            }
        });
}

fn permission_row(
    ui: &mut Ui,
    permission: Permission,
    editing: &Editing,
    set: &PermissionSet,
    cap: &PermissionSet,
    actions: &mut Vec<McpAction>,
) {
    let held_back = matches!(editing, Editing::Client(_))
        && set.contains(permission)
        && !cap.contains(permission);
    ui.horizontal(|ui| {
        ui.set_min_height(44.0);
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.vertical(|ui| {
            ui.set_max_width((ui.available_width() - 56.0 - 12.0).max(120.0));
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                // Only a permission that can change DegenView is marked, with a small amber dot:
                // the many that only look carry nothing, so the marked ones stand out.
                if permission.is_mutating() {
                    let (dot, response) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
                    ui.painter()
                        .circle_filled(dot.center(), 3.0, Tone::Warning.dot());
                    response.on_hover_text("Can change DegenView");
                }
                ui.label(kit::text(
                    permission.title(),
                    12.5,
                    kit::MEDIUM,
                    theme::text_primary(),
                ))
                .on_hover_text(permission.id());
            });
            caption(ui, permission.detail());
            if held_back {
                ui.label(kit::text(
                    "Held back: the maximum does not allow this",
                    kit::size::CAPTION,
                    kit::REGULAR,
                    Tone::Warning.text(),
                ));
            }
            ui.add_space(6.0);
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut on = set.contains(permission);
            let toggled = card::switch(ui, &mut on);
            if toggled {
                actions.push(match editing {
                    Editing::Maximum => McpAction::SetGlobalPermission {
                        permission,
                        allowed: on,
                    },
                    Editing::RemoteCeiling => {
                        let mut ceiling = set.clone();
                        ceiling.set(permission, on);
                        McpAction::SetRemoteCeiling(ceiling)
                    }
                    Editing::Client(id) => McpAction::SetClientPermission {
                        client_id: id.clone(),
                        permission,
                        allowed: on,
                    },
                });
            }
        });
    });
}

fn sheet_footer(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    client: Option<&McpClient>,
    actions: &mut Vec<McpAction>,
    close: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        match client {
            Some(client) if state.confirm_revoke => {
                ui.label(kit::text(
                    format!("Revoke “{}”?", client.name),
                    kit::size::BODY,
                    kit::MEDIUM,
                    theme::text_primary(),
                ));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if kit::destructive_sheet_button(ui, Icon::Xmark, "Revoke").clicked() {
                        actions.push(McpAction::RevokeClient(client.id.clone()));
                        *close = true;
                    }
                    if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                        state.confirm_revoke = false;
                    }
                });
            }
            Some(_) => {
                if kit::destructive_sheet_button(ui, Icon::Xmark, "Revoke…").clicked() {
                    state.confirm_revoke = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if kit::sheet_button(ui, "Done", true, true).clicked() {
                        *close = true;
                    }
                });
            }
            None => {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if kit::sheet_button(ui, "Done", true, true).clicked() {
                        *close = true;
                    }
                });
            }
        }
    });
}
