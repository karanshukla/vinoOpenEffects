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

    let camera = add_camera_group(&page, cmd_tx);
    let processor = add_processor_group(&page, cmd_tx);
    let models = add_models_group(&page);
    add_startup_group(&page, &gui_config);

    (
        page,
        Widgets {
            camera_combo: camera.combo,
            camera_handler: camera.handler,
            camera_ids: camera.ids,
            virtual_cam_label: camera.virtual_cam_label,
            model_rows: models.rows,
            models_group: models.group,
            inference_row: processor.row,
            inference_group: processor.group,
            inference_toggle_auto: processor.toggle_auto,
            inference_toggle_cpu: processor.toggle_cpu,
            inference_toggle_npu: processor.toggle_npu,
            inference_handler: processor.handler,
        },
    )
}

struct CameraGroup {
    combo: adw::ComboRow,
    handler: glib::SignalHandlerId,
    ids: Rc<RefCell<Vec<String>>>,
    virtual_cam_label: gtk::Label,
}

fn add_camera_group(page: &adw::PreferencesPage, cmd_tx: &CmdTx) -> CameraGroup {
    let camera_group = pref_group("Camera", "");
    let combo = adw::ComboRow::builder()
        .title("Source")
        .model(&gtk::StringList::new(&["No cameras found"]))
        .build();
    let ids = Rc::new(RefCell::new(Vec::<String>::new()));
    let handler = {
        let cmd_tx = cmd_tx.clone();
        let ids = Rc::clone(&ids);
        combo.connect_selected_notify(move |row| {
            let ids = ids.borrow();
            if let Some(id) = ids.get(row.selected() as usize) {
                let _ = cmd_tx.send(GuiCommand::SelectCamera { id: id.clone() });
            }
        })
    };
    camera_group.add(&combo);

    let virtual_cam_row = adw::ActionRow::builder()
        .title("Virtual Camera")
        .subtitle("Select this camera in Zoom, OBS, or your browser")
        .build();
    let virtual_cam_label = gtk::Label::builder()
        .label("—")
        .selectable(true)
        .css_classes(["monospace"])
        .build();
    virtual_cam_row.add_suffix(&virtual_cam_label);
    camera_group.add(&virtual_cam_row);
    page.add(&camera_group);

    CameraGroup {
        combo,
        handler,
        ids,
        virtual_cam_label,
    }
}

struct ProcessorGroup {
    row: adw::ActionRow,
    group: adw::ToggleGroup,
    toggle_auto: adw::Toggle,
    toggle_cpu: adw::Toggle,
    toggle_npu: adw::Toggle,
    handler: glib::SignalHandlerId,
}

fn add_processor_group(page: &adw::PreferencesPage, cmd_tx: &CmdTx) -> ProcessorGroup {
    let group_box = pref_group("Processor", "Auto uses the NPU when available.");

    let row = adw::ActionRow::builder()
        .title("Run Models On")
        .subtitle("Using CPU")
        .build();

    let toggle_auto = adw::Toggle::builder().name("auto").label("Auto").build();
    let toggle_cpu = adw::Toggle::builder().name("cpu").label("CPU").build();
    let toggle_npu = adw::Toggle::builder().name("npu").label("NPU").build();

    let group = adw::ToggleGroup::builder()
        .valign(gtk::Align::Center)
        .css_classes(["round"])
        .build();
    group.add(toggle_auto.clone());
    group.add(toggle_cpu.clone());
    group.add(toggle_npu.clone());
    group.set_active_name(Some("auto"));
    group.update_property(&[gtk::accessible::Property::Label("Run models on")]);

    let handler = {
        let cmd_tx = cmd_tx.clone();
        group.connect_active_notify(move |group| {
            if let Some(name) = group.active_name() {
                let _ = cmd_tx.send(GuiCommand::SetInferenceProvider(name.to_string()));
            }
        })
    };

    row.add_suffix(&group);
    group_box.add(&row);
    page.add(&group_box);

    ProcessorGroup {
        row,
        group,
        toggle_auto,
        toggle_cpu,
        toggle_npu,
        handler,
    }
}

struct ModelsGroup {
    group: adw::PreferencesGroup,
    rows: HashMap<&'static str, adw::ActionRow>,
}

fn add_models_group(page: &adw::PreferencesPage) -> ModelsGroup {
    let group = adw::PreferencesGroup::builder().title("Models").build();
    let mut rows = HashMap::new();
    for model in BUNDLED_MODELS {
        let row = adw::ActionRow::builder()
            .title(model.name)
            .subtitle(model.purpose)
            .build();
        let pill = gtk::Label::builder()
            .label("Checking…")
            .css_classes(["dim-label"])
            .build();
        row.add_suffix(&pill);
        unsafe { row.set_data("pill", pill) };
        group.add(&row);
        rows.insert(model.id, row);
    }
    group.add(&delete_models_row());
    page.add(&group);
    ModelsGroup { group, rows }
}

const KEEP_RUNNING_SUBTITLE: &str = "Keep camera effects active after closing this window";
const KEEP_RUNNING_FORCED: &str = "Always on while Start at Login is enabled";
const UNSUPPORTED_SESSION_SUBTITLE: &str =
    "Unsupported in this desktop session. Start it from your compositor config instead.";

fn sync_keep_running_row(row: &adw::SwitchRow, start_at_login: bool) {
    row.set_sensitive(!start_at_login);
    row.set_subtitle(if start_at_login {
        KEEP_RUNNING_FORCED
    } else {
        KEEP_RUNNING_SUBTITLE
    });
}

fn apply_session_support(boot_row: &adw::SwitchRow, supported: bool) {
    if !supported {
        boot_row.set_sensitive(false);
        boot_row.set_subtitle(UNSUPPORTED_SESSION_SUBTITLE);
    }
}

fn add_startup_group(page: &adw::PreferencesPage, gui_config: &Rc<RefCell<GuiConfig>>) {
    let startup = pref_group("Startup", "");

    let keep_running_row = adw::SwitchRow::builder()
        .title("Run in Background")
        .active(gui_config.borrow().keep_running_in_background)
        .build();

    let boot_row = adw::SwitchRow::builder()
        .title("Start at Login")
        .active(shared::systemd::is_enabled())
        .build();
    sync_keep_running_row(&keep_running_row, boot_row.is_active());
    // sway/Hyprland without systemd integration never start the target the unit hangs off.
    apply_session_support(
        &boot_row,
        shared::systemd::session_target_active() != Some(false),
    );

    let keep_running_for_boot = keep_running_row.clone();
    boot_row.connect_active_notify(move |row| {
        let on = row.is_active();
        if let Err(err) = shared::systemd::set_enabled(on) {
            tracing::warn!(%err, "failed to change daemon autostart");
        }
        sync_keep_running_row(&keep_running_for_boot, on);
    });
    startup.add(&boot_row);

    {
        let gui_config = Rc::clone(gui_config);
        keep_running_row.connect_active_notify(move |row| {
            gui_config.borrow_mut().keep_running_in_background = row.is_active();
            gui_config.borrow().save();
        });
    }
    startup.add(&keep_running_row);

    page.add(&startup);
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

fn empty_models_subtitle(has_system_models: bool) -> &'static str {
    if has_system_models {
        "No user models downloaded. System models aren't affected."
    } else {
        "No downloaded models"
    }
}

fn refresh_delete_row(row: &adw::ActionRow, button: &gtk::Button) {
    let (subtitle, can_delete) = match UserPaths::current() {
        Some(paths) if paths.has_models() => {
            (on_disk(paths.size(), &models::tilde(&paths.models)), true)
        }
        paths => {
            let has_system = paths
                .as_ref()
                .is_some_and(|p| !models::system_dirs_with_models(p).is_empty());
            (empty_models_subtitle(has_system).to_string(), false)
        }
    };
    row.set_subtitle(&subtitle);
    button.set_sensitive(can_delete);
}

fn on_disk(bytes: u64, dir: &str) -> String {
    format!("{} in {dir}", glib::format_size(bytes))
}

/// What Delete does, in the confirm dialog.
fn confirm_body(system: &[PathBuf], upgrade: bool) -> String {
    let effect = if system.is_empty() {
        let fetch = if upgrade {
            "run vinoopeneffects-upgrade"
        } else {
            "run scripts/fetch-models.sh from your checkout"
        };
        format!("AI effects will stop working until you download the models again ({fetch}).")
    } else {
        let dirs: Vec<String> = system.iter().map(|d| d.display().to_string()).collect();
        format!(
            "System models in {} will still be used.",
            dirs.join(" and ")
        )
    };
    format!("{effect} The camera will briefly pause; your backgrounds and settings are kept.")
}

fn run_delete(button: &gtk::Button, paths: UserPaths, refresh: impl Fn() + 'static) {
    button.set_sensitive(false);
    let button = button.clone();
    glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(move || models::delete(&paths))
            .await
            .unwrap_or_else(|_| Err("Deleting models did not finish.".into()));
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
                let alert = adw::AlertDialog::new(Some("Could Not Delete Models"), Some(&why));
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
        assert!(!release.contains("daemon"));
        let checkout = confirm_body(&[], false);
        assert!(checkout.contains("scripts/fetch-models.sh"));
    }

    #[test]
    fn confirm_says_system_models_stay() {
        let body = confirm_body(
            &[PathBuf::from("/usr/local/share/vinoopeneffects/models")],
            true,
        );
        assert!(body.contains("/usr/local/share/vinoopeneffects/models will still be used"));
        assert!(!body.contains("stop working"));
        assert!(!body.contains("daemon"));
    }

    #[test]
    fn empty_subtitle_mentions_system_models_only_when_present() {
        assert_eq!(empty_models_subtitle(false), "No downloaded models");
        assert!(empty_models_subtitle(true).contains("System models"));
    }

    #[test]
    fn on_disk_shows_size_and_place() {
        assert_eq!(
            on_disk(8_400_000, "~/.local/share/vinoopeneffects/models"),
            "8.4\u{a0}MB in ~/.local/share/vinoopeneffects/models"
        );
    }
}

#[cfg(test)]
mod widget_tests {
    use super::*;
    use crate::gtk_test::{channel_pair, children, drain, run};

    fn group_titles(page: &adw::PreferencesPage) -> Vec<String> {
        let mut titles = Vec::new();
        let mut stack = children(page);
        stack.reverse();
        while let Some(widget) = stack.pop() {
            if let Some(group) = widget.downcast_ref::<adw::PreferencesGroup>() {
                titles.push(group.title().to_string());
            } else {
                let mut inner = children(&widget);
                inner.reverse();
                stack.extend(inner);
            }
        }
        titles
    }

    #[test]
    fn the_camera_group_has_a_source_combo_and_a_virtual_camera_readout() {
        run(|| {
            let (tx, _rx) = channel_pair();
            let page = adw::PreferencesPage::new();
            let group = add_camera_group(&page, &tx);
            assert_eq!(group.combo.title(), "Source");
            assert_eq!(group.combo.model().unwrap().n_items(), 1);
            assert_eq!(group.virtual_cam_label.label(), "—");
            assert!(group.virtual_cam_label.is_selectable());
            assert!(group.ids.borrow().is_empty());
            assert_eq!(group_titles(&page), ["Camera"]);
        });
    }

    #[test]
    fn choosing_a_source_sends_the_camera_id_at_that_position() {
        run(|| {
            let (tx, mut rx) = channel_pair();
            let page = adw::PreferencesPage::new();
            let group = add_camera_group(&page, &tx);
            *group.ids.borrow_mut() = vec!["cam-a".into(), "cam-b".into()];
            group
                .combo
                .set_model(Some(&gtk::StringList::new(&["A", "B"])));
            drain(&mut rx);
            group.combo.set_selected(1);
            let sent = drain(&mut rx);
            assert!(
                matches!(sent.last(), Some(GuiCommand::SelectCamera { id }) if id == "cam-b"),
                "{sent:?}"
            );
        });
    }

    #[test]
    fn the_processor_group_offers_auto_cpu_npu_with_auto_selected() {
        run(|| {
            let (tx, _rx) = channel_pair();
            let page = adw::PreferencesPage::new();
            let group = add_processor_group(&page, &tx);
            assert_eq!(group.row.title(), "Run Models On");
            assert_eq!(group.group.n_toggles(), 3);
            for (i, name) in ["auto", "cpu", "npu"].iter().enumerate() {
                let toggle = group.group.toggle(i as u32).unwrap();
                assert_eq!(toggle.name().as_str(), *name);
            }
            assert_eq!(group.toggle_auto.label().as_deref(), Some("Auto"));
            assert_eq!(group.toggle_cpu.label().as_deref(), Some("CPU"));
            assert_eq!(group.toggle_npu.label().as_deref(), Some("NPU"));
            assert_eq!(group.group.active_name().as_deref(), Some("auto"));
            assert_eq!(group_titles(&page), ["Processor"]);
        });
    }

    #[test]
    fn choosing_a_processor_sends_its_name() {
        run(|| {
            let (tx, mut rx) = channel_pair();
            let page = adw::PreferencesPage::new();
            let group = add_processor_group(&page, &tx);
            group.group.set_active_name(Some("npu"));
            let sent = drain(&mut rx);
            assert!(
                matches!(sent.as_slice(), [GuiCommand::SetInferenceProvider(p)] if p == "npu"),
                "{sent:?}"
            );
        });
    }

    fn without_entities(markup: &str) -> String {
        markup
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&#39;", "'")
            .replace("&amp;", "&")
    }

    #[test]
    fn the_models_group_lists_each_bundled_model_then_the_delete_row() {
        run(|| {
            let page = adw::PreferencesPage::new();
            let models = add_models_group(&page);
            assert_eq!(models.group.title(), "Models");
            assert_eq!(models.rows.len(), BUNDLED_MODELS.len());
            for model in BUNDLED_MODELS {
                let row = &models.rows[model.id];
                assert_eq!(row.title(), model.name);
                // libadwaita versions differ on whether the row hands markup back escaped.
                let shown = row.subtitle().unwrap();
                assert_eq!(without_entities(&shown), without_entities(model.purpose));
            }
            assert_eq!(group_titles(&page), ["Models"]);
        });
    }

    #[test]
    fn the_startup_group_has_the_background_row_after_the_login_row() {
        run(|| {
            let page = adw::PreferencesPage::new();
            let config = Rc::new(RefCell::new(GuiConfig::default()));
            add_startup_group(&page, &config);
            assert_eq!(group_titles(&page), ["Startup"]);
            let mut titles = Vec::new();
            let mut stack = children(&page);
            while let Some(widget) = stack.pop() {
                if let Some(row) = widget.downcast_ref::<adw::SwitchRow>() {
                    titles.push(row.title().to_string());
                } else {
                    stack.extend(children(&widget));
                }
            }
            titles.sort();
            assert_eq!(titles, ["Run in Background", "Start at Login"]);
        });
    }

    #[test]
    fn the_page_holds_camera_processor_models_startup_in_that_order() {
        run(|| {
            let (tx, _rx) = channel_pair();
            let (page, widgets) = build_page(&tx, Rc::new(RefCell::new(GuiConfig::default())));
            assert_eq!(
                group_titles(&page),
                ["Camera", "Processor", "Models", "Startup"]
            );
            assert_eq!(widgets.model_rows.len(), BUNDLED_MODELS.len());
        });
    }

    #[test]
    fn start_at_login_disables_run_in_background_and_explains_why() {
        run(|| {
            let row = adw::SwitchRow::builder().title("Run in Background").build();
            sync_keep_running_row(&row, false);
            assert!(row.is_sensitive());
            assert_eq!(row.subtitle().as_deref(), Some(KEEP_RUNNING_SUBTITLE));

            sync_keep_running_row(&row, true);
            assert!(!row.is_sensitive());
            assert_eq!(row.subtitle().as_deref(), Some(KEEP_RUNNING_FORCED));
        });
    }

    #[test]
    fn unsupported_sessions_disable_start_at_login() {
        run(|| {
            let row = adw::SwitchRow::builder().title("Start at Login").build();
            apply_session_support(&row, true);
            assert!(row.is_sensitive());
            assert!(row.subtitle().as_deref().is_none_or(str::is_empty));

            apply_session_support(&row, false);
            assert!(!row.is_sensitive());
            assert_eq!(
                row.subtitle().as_deref(),
                Some(UNSUPPORTED_SESSION_SUBTITLE)
            );
        });
    }
}
