//! The Preferences dialog (`AdwPreferencesDialog`): camera selection, the
//! virtual-camera node readout, and the bundled-model library with a way to
//! delete the downloaded models. Built once and re-presented from the primary
//! menu.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use std::path::PathBuf;

use adw::prelude::*;
use adw::{gio, glib};

use crate::config::GuiConfig;
use crate::constants::BUNDLED_MODELS;
use crate::dbus_client::{CmdTx, GuiCommand};
use crate::models::{self, UserPaths};
use crate::widgets::pref_group;

/// Rows the state-sync layer updates when the daemon's state changes.
pub struct Widgets {
    /// Physical-camera picker.
    pub camera_combo: adw::ComboRow,
    pub camera_handler: glib::SignalHandlerId,
    /// Camera ids in the same order as `camera_combo`'s model, shared with its
    /// change handler so a selection resolves back to an id.
    pub camera_ids: Rc<RefCell<Vec<String>>>,
    /// Node name of the virtual camera, shown on the "Virtual Camera" row.
    pub virtual_cam_label: gtk::Label,
    pub model_rows: HashMap<&'static str, adw::ActionRow>,
    /// Says how to fetch models, only while some are missing.
    pub models_group: adw::PreferencesGroup,
    pub inference_row: adw::ActionRow,
    pub inference_group: adw::ToggleGroup,
    #[allow(dead_code)]
    pub inference_toggle_auto: adw::Toggle,
    #[allow(dead_code)]
    pub inference_toggle_cpu: adw::Toggle,
    pub inference_toggle_npu: adw::Toggle,
    pub inference_handler: glib::SignalHandlerId,
}

pub fn build(
    cmd_tx: &CmdTx,
    gui_config: Rc<RefCell<GuiConfig>>,
) -> (adw::PreferencesDialog, Widgets) {
    let dialog = adw::PreferencesDialog::builder()
        .title("Preferences")
        .build();
    let (page, widgets) = build_page(cmd_tx, gui_config);
    dialog.add(&page);
    (dialog, widgets)
}

/// The dialog's one page, separate so tests can walk it: an unpresented
/// dialog doesn't parent its pages.
pub(crate) fn build_page(
    cmd_tx: &CmdTx,
    gui_config: Rc<RefCell<GuiConfig>>,
) -> (adw::PreferencesPage, Widgets) {
    let page = adw::PreferencesPage::builder()
        .title("General")
        .icon_name("preferences-system-symbolic")
        .build();

    let camera_group = pref_group(
        "Camera",
        "Choose the physical camera vinoOpenEffects captures from",
    );
    let camera_combo = adw::ComboRow::builder()
        .title("Source")
        .model(&gtk::StringList::new(&["No cameras found"]))
        .build();
    let camera_ids = Rc::new(RefCell::new(Vec::<String>::new()));
    let camera_handler = {
        let cmd_tx = cmd_tx.clone();
        let camera_ids = Rc::clone(&camera_ids);
        camera_combo.connect_selected_notify(move |row| {
            let ids = camera_ids.borrow();
            if let Some(id) = ids.get(row.selected() as usize) {
                let _ = cmd_tx.send(GuiCommand::SelectCamera { id: id.clone() });
            }
        })
    };
    camera_group.add(&camera_combo);

    let virtual_cam_row = adw::ActionRow::builder()
        .title("Virtual Camera")
        .subtitle("Pick this one in Zoom, OBS or your browser")
        .build();
    let virtual_cam_label = gtk::Label::builder()
        .label("—")
        .selectable(true)
        .css_classes(["monospace"])
        .build();
    virtual_cam_row.add_suffix(&virtual_cam_label);
    camera_group.add(&virtual_cam_row);
    page.add(&camera_group);

    let inference_group_box = pref_group(
        "Processor",
        "Where the effect models run. Auto picks the NPU when there is one",
    );

    let inference_row = adw::ActionRow::builder()
        .title("Run Models On")
        .subtitle("Using the CPU")
        .build();

    let inference_toggle_auto = adw::Toggle::builder().name("auto").label("Auto").build();
    let inference_toggle_cpu = adw::Toggle::builder().name("cpu").label("CPU").build();
    let inference_toggle_npu = adw::Toggle::builder().name("npu").label("NPU").build();

    let inference_group = adw::ToggleGroup::builder()
        .valign(gtk::Align::Center)
        .css_classes(["round"])
        .build();
    inference_group.add(inference_toggle_auto.clone());
    inference_group.add(inference_toggle_cpu.clone());
    inference_group.add(inference_toggle_npu.clone());
    inference_group.set_active_name(Some("auto"));
    inference_group.update_property(&[gtk::accessible::Property::Label("Run models on")]);

    let inference_handler = {
        let cmd_tx = cmd_tx.clone();
        inference_group.connect_active_notify(move |group| {
            if let Some(name) = group.active_name() {
                let _ = cmd_tx.send(GuiCommand::SetInferenceProvider(name.to_string()));
            }
        })
    };

    inference_row.add_suffix(&inference_group);
    inference_group_box.add(&inference_row);
    page.add(&inference_group_box);

    let models_group = adw::PreferencesGroup::builder().title("Models").build();
    let mut model_rows = HashMap::new();
    for (id, name, purpose) in BUNDLED_MODELS {
        let row = adw::ActionRow::builder()
            .title(*name)
            .subtitle(*purpose)
            .build();
        let pill = gtk::Label::builder()
            .label("Checking…")
            .css_classes(["dim-label"])
            .build();
        row.add_suffix(&pill);
        unsafe { row.set_data("pill", pill) };
        models_group.add(&row);
        model_rows.insert(*id, row);
    }
    models_group.add(&delete_models_row());
    page.add(&models_group);

    let startup = pref_group("Startup", "Control when the vinoOpenEffects daemon runs");

    let keep_running_row = adw::SwitchRow::builder()
        .title("Run in Background")
        .subtitle("Leave the daemon on after this window closes")
        .active(gui_config.borrow().keep_running_in_background)
        .build();

    let boot_row = adw::SwitchRow::builder()
        .title("Start at Login")
        .subtitle("Start the daemon when you log in")
        .active(shared::systemd::is_enabled())
        .build();
    keep_running_row.set_sensitive(!boot_row.is_active());
    // sway/Hyprland without systemd integration never start the target the unit hangs off.
    if shared::systemd::session_target_active() == Some(false) {
        boot_row.set_subtitle(
            "Your session doesn't start graphical-session.target, so this won't take effect. \
             Start the daemon from your compositor's config instead",
        );
    }

    let keep_running_for_boot = keep_running_row.clone();
    boot_row.connect_active_notify(move |row| {
        let on = row.is_active();
        if let Err(err) = shared::systemd::set_enabled(on) {
            tracing::warn!(%err, "failed to change daemon autostart");
        }
        keep_running_for_boot.set_sensitive(!on);
    });
    startup.add(&boot_row);

    {
        let gui_config = Rc::clone(&gui_config);
        keep_running_row.connect_active_notify(move |row| {
            gui_config.borrow_mut().keep_running_in_background = row.is_active();
            gui_config.borrow().save();
        });
    }
    startup.add(&keep_running_row);

    page.add(&startup);

    (
        page,
        Widgets {
            camera_combo,
            camera_handler,
            camera_ids,
            virtual_cam_label,
            model_rows,
            models_group,
            inference_row,
            inference_group,
            inference_toggle_auto,
            inference_toggle_cpu,
            inference_toggle_npu,
            inference_handler,
        },
    )
}

/// "Downloaded Models": what's on disk in the user's home, and Delete…, which
/// confirms in an `AdwAlertDialog` like Check for Updates does.
fn delete_models_row() -> adw::ActionRow {
    let row = adw::ActionRow::builder().title("Downloaded Models").build();
    let button = gtk::Button::builder()
        .label("Delete…")
        .valign(gtk::Align::Center)
        .css_classes(["destructive-action"])
        .build();
    row.add_suffix(&button);

    let refresh = {
        let row = row.clone();
        let button = button.clone();
        move || refresh_delete_row(&row, &button)
    };
    refresh();
    // The installer or a terminal can change the directory while the dialog is closed.
    row.connect_map({
        let refresh = refresh.clone();
        move |_| refresh()
    });

    button.connect_clicked(move |button| {
        let Some(paths) = UserPaths::current() else {
            return;
        };
        let system = models::system_dirs_with_models(&paths);
        let upgrade = crate::update::upgrade_command().is_some();
        let dialog = adw::AlertDialog::new(
            Some("Delete Models?"),
            Some(&confirm_body(&system, upgrade)),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("delete", "Delete");
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let target = button.clone();
        let refresh = refresh.clone();
        dialog.connect_response(Some("delete"), move |_, _| {
            run_delete(&target, paths.clone(), refresh.clone());
        });
        dialog.present(Some(button));
    });
    row
}

fn refresh_delete_row(row: &adw::ActionRow, button: &gtk::Button) {
    let paths = UserPaths::current();
    let (subtitle, can_delete) = match &paths {
        Some(paths) if paths.has_models() => {
            (on_disk(paths.size(), &models::tilde(&paths.models)), true)
        }
        _ => (
            "None downloaded. Models installed system-wide stay where they are".to_string(),
            false,
        ),
    };
    row.set_subtitle(&subtitle);
    button.set_sensitive(can_delete);
}

fn on_disk(bytes: u64, dir: &str) -> String {
    format!("{} in {dir}", glib::format_size(bytes))
}

/// What Delete does, in the confirm dialog.
fn confirm_body(system: &[PathBuf], upgrade: bool) -> String {
    let fetch = if upgrade {
        "run vinoopeneffects-upgrade"
    } else {
        "run scripts/fetch-models.sh from your checkout"
    };
    let effect = if system.is_empty() {
        format!(
            "Portrait blur, background replace, Center Stage and gesture reactions stop \
             working until you download the models again: {fetch}."
        )
    } else {
        let dirs: Vec<String> = system.iter().map(|d| d.display().to_string()).collect();
        format!(
            "The models installed system-wide in {} stay, and the effects use those. \
             To download your own copy again, {fetch}.",
            dirs.join(" and ")
        )
    };
    format!(
        "{effect} The effects daemon restarts, so the camera pauses for a moment. \
         Your backgrounds and settings are kept."
    )
}

fn run_delete(button: &gtk::Button, paths: UserPaths, refresh: impl Fn() + 'static) {
    button.set_sensitive(false);
    let button = button.clone();
    glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(move || models::delete(&paths))
            .await
            .unwrap_or_else(|_| Err("The delete stopped before it finished.".into()));
        refresh();
        match result {
            Ok(()) => {
                if let Some(prefs) = button.ancestor(adw::PreferencesDialog::static_type()) {
                    prefs
                        .downcast::<adw::PreferencesDialog>()
                        .expect("ancestor of that type")
                        .add_toast(adw::Toast::new("Models deleted"));
                }
            }
            Err(why) => {
                let alert = adw::AlertDialog::new(Some("Couldn't Delete Models"), Some(&why));
                alert.add_response("close", "Close");
                alert.set_close_response("close");
                alert.present(Some(&button));
            }
        }
    });
}

#[cfg(test)]
mod delete_tests {
    use super::*;

    #[test]
    fn confirm_says_how_to_get_them_back() {
        let release = confirm_body(&[], true);
        assert!(release.contains("vinoopeneffects-upgrade"));
        assert!(release.contains("stop working"));
        assert!(release.contains("backgrounds and settings are kept"));
        let checkout = confirm_body(&[], false);
        assert!(checkout.contains("scripts/fetch-models.sh"));
    }

    #[test]
    fn confirm_says_system_models_stay() {
        let body = confirm_body(
            &[PathBuf::from("/usr/local/share/vinoopeneffects/models")],
            true,
        );
        assert!(body.contains("/usr/local/share/vinoopeneffects/models stay"));
        assert!(!body.contains("stop working"));
    }

    #[test]
    fn on_disk_shows_size_and_place() {
        assert_eq!(
            on_disk(8_400_000, "~/.local/share/vinoopeneffects/models"),
            "8.4\u{a0}MB in ~/.local/share/vinoopeneffects/models"
        );
    }
}
