//! The live preview: the daemon's virtual camera through `pipewiresrc` into a
//! `gtk::Picture`. The pipeline runs only while the preview is mapped, so
//! hiding the window releases the real camera. Floating over it: the camera
//! picker, a "live" pill, the compare button and the node name.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gst::prelude::*;
use gstreamer as gst;
use gtk::glib;

use crate::constants::VCAM_NODE_NAME;
use crate::dbus_client::{CmdTx, GuiCommand};

/// How long to wait for the first frame before saying something is wrong.
/// Covers the camera opening and the models loading on first use; a missing
/// PipeWire target would otherwise sit silent for pipewiresrc's own 30 s.
const NO_FRAMES_TIMEOUT_SECS: u32 = 10;
const NODE_REREGISTER_SECS: u32 = 2;

pub struct Preview {
    /// The framed preview; place this in the window.
    pub frame: gtk::Overlay,
    /// Physical-camera picker. Its model is filled by the state-sync layer.
    pub camera_dropdown: gtk::DropDown,
    pub camera_handler: glib::SignalHandlerId,
    /// Press-and-hold shows the camera without effects; wired by the window.
    pub compare: gtk::Button,
    /// "Virtual camera live" text; shortened at narrow widths.
    pub live_label: gtk::Label,
    /// Virtual camera node-name pill.
    pub node_pill: gtk::Label,
    picture: gtk::Picture,
    status: gtk::Label,
    live_pill: gtk::Box,
    /// Centred "can't reach the virtual camera" notice with its actions.
    problem: gtk::Box,
    problem_text: gtk::Label,
    retry_btn: gtk::Button,
    restart_btn: gtk::Button,
    /// Set once the sink has produced a frame for the current attempt.
    has_frames: Rc<Cell<bool>>,
    /// Bumped on every start/stop so a stale no-frames timer does nothing.
    attempt: Cell<u32>,
    bus_watch: RefCell<Option<gst::bus::BusWatchGuard>>,
    /// Built on first map and reused; rebuilt after a failed attempt.
    pipeline: RefCell<Option<gst::Pipeline>>,
}

/// Build the preview. `camera_ids` holds the camera ids in the same order as
/// the picker's model, so a selection resolves back to an id.
pub fn build(cmd_tx: &CmdTx, camera_ids: Rc<RefCell<Vec<String>>>) -> Rc<Preview> {
    let picture = build_picture();
    let status = build_status_label();
    let ProblemNotice {
        container: problem,
        text: problem_text,
        retry_btn,
        restart_btn,
    } = build_problem_notice();
    let (camera_dropdown, camera_handler) = build_camera_dropdown(cmd_tx, camera_ids);
    let (live_pill, live_label) = build_live_pill();
    let compare = build_compare_button();
    let node_pill = build_node_pill();

    let frame = gtk::Overlay::builder()
        .child(&picture)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["oe-preview-frame"])
        .build();
    frame.add_overlay(&status);
    frame.add_overlay(&problem);
    frame.add_overlay(&camera_dropdown);
    frame.add_overlay(&live_pill);
    frame.add_overlay(&compare);
    frame.add_overlay(&node_pill);

    let preview = Rc::new(Preview {
        frame,
        camera_dropdown,
        camera_handler,
        compare,
        live_label,
        node_pill,
        picture,
        status,
        live_pill,
        problem,
        problem_text,
        retry_btn,
        restart_btn,
        has_frames: Rc::new(Cell::new(false)),
        attempt: Cell::new(0),
        bus_watch: RefCell::new(None),
        pipeline: RefCell::new(None),
    });

    connect_problem_actions(&preview);
    connect_map_lifecycle(&preview);

    preview
}

fn build_picture() -> gtk::Picture {
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .hexpand(true)
        .vexpand(true)
        .build();
    picture.update_property(&[
        gtk::accessible::Property::Label("Camera preview"),
        gtk::accessible::Property::Description("Virtual camera output with effects applied"),
    ]);
    picture
}

fn build_status_label() -> gtk::Label {
    gtk::Label::builder()
        .label("Connecting…")
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["oe-preview-status"])
        .build()
}

/// The centred "can't reach the virtual camera" notice and its two buttons.
struct ProblemNotice {
    container: gtk::Box,
    text: gtk::Label,
    retry_btn: gtk::Button,
    restart_btn: gtk::Button,
}

fn build_problem_notice() -> ProblemNotice {
    let text = gtk::Label::builder()
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["oe-preview-status"])
        .build();
    let retry_btn = gtk::Button::builder()
        .label("Try Again")
        .css_classes(["oe-pill-button"])
        .build();
    let restart_btn = gtk::Button::builder()
        .label("Restart Camera Services")
        .tooltip_text("Restarts WirePlumber and the camera service. Other apps may briefly lose camera or audio.")
        .css_classes(["oe-pill-button"])
        .build();
    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .halign(gtk::Align::Center)
        .build();
    actions.append(&retry_btn);
    actions.append(&restart_btn);
    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .visible(false)
        .build();
    container.append(&text);
    container.append(&actions);
    ProblemNotice {
        container,
        text,
        retry_btn,
        restart_btn,
    }
}

fn build_camera_dropdown(
    cmd_tx: &CmdTx,
    camera_ids: Rc<RefCell<Vec<String>>>,
) -> (gtk::DropDown, glib::SignalHandlerId) {
    let camera_dropdown = gtk::DropDown::builder()
        .model(&gtk::StringList::new(&["No cameras found"]))
        .tooltip_text("Camera")
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .margin_start(14)
        .margin_top(14)
        .css_classes(["oe-pill-dropdown"])
        .build();
    camera_dropdown.update_property(&[gtk::accessible::Property::Label("Camera")]);
    let camera_handler = {
        let cmd_tx = cmd_tx.clone();
        camera_dropdown.connect_selected_notify(move |dd| {
            let ids = camera_ids.borrow();
            if let Some(id) = ids.get(dd.selected() as usize) {
                let _ = cmd_tx.send(GuiCommand::SelectCamera { id: id.clone() });
            }
        })
    };
    (camera_dropdown, camera_handler)
}

fn build_live_pill() -> (gtk::Box, gtk::Label) {
    let live_label = gtk::Label::new(Some("Virtual camera live"));
    let live_pill = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .margin_end(14)
        .margin_top(14)
        .visible(false)
        .css_classes(["oe-pill"])
        .build();
    live_pill.append(
        &gtk::Box::builder()
            .valign(gtk::Align::Center)
            .css_classes(["oe-live-dot"])
            .build(),
    );
    live_pill.append(&live_label);
    (live_pill, live_label)
}

fn build_compare_button() -> gtk::Button {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    content.append(
        &gtk::Image::builder()
            .icon_name("object-flip-horizontal-symbolic")
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build(),
    );
    content.append(&gtk::Label::new(Some("Hold to Compare")));
    gtk::Button::builder()
        .child(&content)
        .tooltip_text("View original camera feed")
        .halign(gtk::Align::Start)
        .valign(gtk::Align::End)
        .margin_start(14)
        .margin_bottom(14)
        .css_classes(["oe-pill-button"])
        .build()
}

fn build_node_pill() -> gtk::Label {
    gtk::Label::builder()
        .label(VCAM_NODE_NAME)
        .tooltip_text("Select this camera in Zoom, OBS, or your browser")
        .halign(gtk::Align::End)
        .valign(gtk::Align::End)
        .margin_end(14)
        .margin_bottom(14)
        .css_classes(["oe-pill", "monospace"])
        .build()
}

fn connect_problem_actions(preview: &Rc<Preview>) {
    let weak = Rc::downgrade(preview);
    preview.retry_btn.connect_clicked(move |_| {
        if let Some(preview) = weak.upgrade() {
            preview.restart_pipeline();
        }
    });
    let weak = Rc::downgrade(preview);
    preview.restart_btn.connect_clicked(move |_| {
        if let Some(preview) = weak.upgrade() {
            preview.restart_services();
        }
    });
}

fn connect_map_lifecycle(preview: &Rc<Preview>) {
    let preview_map = Rc::downgrade(preview);
    preview.frame.connect_map(move |_| {
        if let Some(preview) = preview_map.upgrade() {
            preview.start();
        }
    });
    let preview_unmap = Rc::downgrade(preview);
    preview.frame.connect_unmap(move |_| {
        if let Some(preview) = preview_unmap.upgrade() {
            preview.stop();
        }
    });
}

impl Preview {
    fn start(self: &Rc<Self>) {
        self.status.set_label("Connecting…");
        self.status.set_visible(true);
        self.live_pill.set_visible(false);
        self.problem.set_visible(false);
        self.has_frames.set(false);

        let attempt = self.attempt.get().wrapping_add(1);
        self.attempt.set(attempt);
        let weak = Rc::downgrade(self);
        glib::timeout_add_seconds_local_once(NO_FRAMES_TIMEOUT_SECS, move || {
            if let Some(preview) = weak.upgrade()
                && preview.attempt.get() == attempt
                && !preview.has_frames.get()
            {
                preview.show_problem("No video from the virtual camera yet");
            }
        });

        if self.pipeline.borrow().is_none() {
            match self.build_pipeline() {
                Ok(pipeline) => {
                    self.watch_errors(&pipeline);
                    *self.pipeline.borrow_mut() = Some(pipeline);
                }
                Err(err) => {
                    self.show_problem(
                        &missing_plugin().unwrap_or(format!("Preview unavailable: {err}")),
                    );
                    return;
                }
            }
        }

        if let Some(pipeline) = self.pipeline.borrow().as_ref()
            && let Err(err) = pipeline.set_state(gst::State::Playing)
        {
            self.show_problem(&format!("Could not start preview: {err}"));
        }
    }

    /// Show a pipeline error (usually "target not found") instead of "Connecting…".
    fn watch_errors(self: &Rc<Self>, pipeline: &gst::Pipeline) {
        let Some(bus) = pipeline.bus() else { return };
        let weak = Rc::downgrade(self);
        let guard = bus.add_watch_local(move |_, msg| {
            if let gst::MessageView::Error(err) = msg.view()
                && let Some(preview) = weak.upgrade()
            {
                preview.on_error(&err.error().to_string());
            }
            glib::ControlFlow::Continue
        });
        if let Ok(guard) = guard {
            *self.bus_watch.borrow_mut() = Some(guard);
        }
    }

    fn on_error(&self, message: &str) {
        self.drop_pipeline();
        self.show_problem(&if message.contains("target not found") {
            "PipeWire cannot find the virtual camera. Restart the camera service.".to_string()
        } else {
            format!("Preview error: {message}")
        });
    }

    fn show_problem(&self, text: &str) {
        // Or the no-frames timer replaces a specific reason with a vague one.
        self.attempt.set(self.attempt.get().wrapping_add(1));
        self.status.set_visible(false);
        self.live_pill.set_visible(false);
        self.problem_text.set_label(text);
        self.retry_btn.set_sensitive(true);
        self.restart_btn.set_sensitive(true);
        self.problem.set_visible(true);
        self.frame
            .announce(text, gtk::AccessibleAnnouncementPriority::Medium);
    }

    pub(crate) fn drop_pipeline(&self) {
        self.bus_watch.borrow_mut().take();
        if let Some(pipeline) = self.pipeline.borrow_mut().take() {
            let _ = pipeline.set_state(gst::State::Null);
        }
    }

    fn restart_pipeline(self: &Rc<Self>) {
        self.drop_pipeline();
        self.start();
    }

    /// Restart the services the preview depends on: WirePlumber (which links
    /// consumers to the virtual camera) and our daemon. Then reconnect.
    fn restart_services(self: &Rc<Self>) {
        self.drop_pipeline();
        self.attempt.set(self.attempt.get().wrapping_add(1));
        self.problem_text.set_label("Restarting camera services…");
        self.frame.announce(
            "Restarting camera services",
            gtk::AccessibleAnnouncementPriority::Low,
        );
        self.retry_btn.set_sensitive(false);
        self.restart_btn.set_sensitive(false);

        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = gtk::gio::spawn_blocking(|| {
                std::process::Command::new("systemctl")
                    .args(["--user", "restart", "wireplumber", "vinoopeneffects-daemon"])
                    .output()
            })
            .await;
            glib::timeout_future_seconds(NODE_REREGISTER_SECS).await;
            let Some(preview) = weak.upgrade() else {
                return;
            };
            match result {
                Ok(Ok(out)) if out.status.success() => preview.restart_pipeline(),
                Ok(Ok(out)) => preview.show_problem(&format!(
                    "Restart failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                )),
                _ => preview.show_problem("Could not run systemctl to restart the services."),
            }
        });
    }

    fn stop(&self) {
        self.attempt.set(self.attempt.get().wrapping_add(1));
        if let Some(pipeline) = self.pipeline.borrow().as_ref() {
            let _ = pipeline.set_state(gst::State::Null);
        }
        self.live_pill.set_visible(false);
    }

    /// `pipewiresrc target-object=vinoopeneffects ! videoconvert ! gtk4paintablesink`,
    /// wiring the sink's paintable into the `Picture`.
    fn build_pipeline(&self) -> Result<gst::Pipeline, glib::BoolError> {
        let src = gst::ElementFactory::make("pipewiresrc")
            .property("target-object", VCAM_NODE_NAME)
            .build()?;
        let convert = gst::ElementFactory::make("videoconvert").build()?;
        let sink = gst::ElementFactory::make("gtk4paintablesink").build()?;

        let paintable = sink.property::<gtk::gdk::Paintable>("paintable");
        self.picture.set_paintable(Some(&paintable));

        let status = self.status.downgrade();
        let live_pill = self.live_pill.downgrade();
        let problem = self.problem.downgrade();
        let frame = self.frame.downgrade();
        let has_frames = self.has_frames.clone();
        paintable.connect_invalidate_contents(move |_| {
            if !has_frames.replace(true)
                && let Some(frame) = frame.upgrade()
            {
                frame.announce("Preview live", gtk::AccessibleAnnouncementPriority::Low);
            }
            if let Some(problem) = problem.upgrade() {
                problem.set_visible(false);
            }
            if let Some(status) = status.upgrade() {
                status.set_visible(false);
            }
            if let Some(live_pill) = live_pill.upgrade() {
                live_pill.set_visible(true);
            }
        });

        let pipeline = gst::Pipeline::new();
        pipeline.add_many([&src, &convert, &sink])?;
        gst::Element::link_many([&src, &convert, &sink])?;

        Ok(pipeline)
    }
}

/// Which GStreamer plugin the preview lacks, in words, with the package that
/// has it (`docs/install.md`).
fn missing_plugin() -> Option<String> {
    let (what, packages) = [
        (
            "pipewiresrc",
            "PipeWire",
            "pipewire-gstreamer, gstreamer1.0-pipewire or gst-plugin-pipewire",
        ),
        (
            "videoconvert",
            "video convert",
            "gstreamer1-plugins-base or gstreamer1.0-plugins-base",
        ),
        (
            "gtk4paintablesink",
            "GTK 4",
            "gstreamer1-plugin-gtk4, gstreamer1.0-gtk4 or gst-plugin-gtk4",
        ),
    ]
    .into_iter()
    .find(|(element, _, _)| gst::ElementFactory::find(element).is_none())
    .map(|(_, what, packages)| (what, packages))?;
    Some(format!("Missing GStreamer {what} plugin ({packages})."))
}

#[cfg(test)]
mod widget_tests {
    use super::*;
    use crate::gtk_test::{channel_pair, children, drain, run, type_names};

    #[test]
    fn the_picture_fills_the_frame_and_is_named_for_screen_readers() {
        run(|| {
            let picture = build_picture();
            assert_eq!(picture.content_fit(), gtk::ContentFit::Cover);
            assert!(picture.hexpands() && picture.vexpands());
            assert!(gtk::test_accessible_has_property(
                &picture,
                gtk::AccessibleProperty::Label
            ));
            assert!(gtk::test_accessible_has_property(
                &picture,
                gtk::AccessibleProperty::Description
            ));
        });
    }

    #[test]
    fn the_status_line_starts_as_connecting() {
        run(|| {
            let status = build_status_label();
            assert_eq!(status.label(), "Connecting…");
            assert!(status.wraps());
            assert!(status.has_css_class("oe-preview-status"));
        });
    }

    #[test]
    fn the_problem_notice_starts_hidden_with_text_then_two_buttons() {
        run(|| {
            let notice = build_problem_notice();
            assert!(!notice.container.is_visible());
            assert_eq!(notice.container.orientation(), gtk::Orientation::Vertical);
            assert_eq!(notice.text.label(), "");

            let rows = children(&notice.container);
            assert_eq!(type_names(&rows), ["GtkLabel", "GtkBox"]);
            assert_eq!(rows[0], notice.text.clone().upcast::<gtk::Widget>());
            let buttons = children(&rows[1]);
            assert_eq!(buttons[0], notice.retry_btn.clone().upcast::<gtk::Widget>());
            assert_eq!(
                buttons[1],
                notice.restart_btn.clone().upcast::<gtk::Widget>()
            );
            assert_eq!(notice.retry_btn.label().as_deref(), Some("Try Again"));
            assert_eq!(
                notice.restart_btn.label().as_deref(),
                Some("Restart Camera Services")
            );
            let warning = notice.restart_btn.tooltip_text().unwrap();
            assert!(warning.contains("WirePlumber"), "{warning}");
        });
    }

    #[test]
    fn the_camera_dropdown_starts_with_a_placeholder_and_is_named() {
        run(|| {
            let (tx, _rx) = channel_pair();
            let (dropdown, _handler) = build_camera_dropdown(&tx, Rc::default());
            assert_eq!(dropdown.model().unwrap().n_items(), 1);
            assert_eq!(dropdown.tooltip_text().as_deref(), Some("Camera"));
            assert!(dropdown.has_css_class("oe-pill-dropdown"));
            assert!(gtk::test_accessible_has_property(
                &dropdown,
                gtk::AccessibleProperty::Label
            ));
        });
    }

    #[test]
    fn picking_a_camera_sends_the_id_at_that_position() {
        run(|| {
            let (tx, mut rx) = channel_pair();
            let ids = Rc::new(RefCell::new(vec!["cam-a".to_string(), "cam-b".to_string()]));
            let (dropdown, _handler) = build_camera_dropdown(&tx, ids);
            dropdown.set_model(Some(&gtk::StringList::new(&["A", "B"])));
            drain(&mut rx);
            dropdown.set_selected(1);
            let sent = drain(&mut rx);
            assert!(
                matches!(sent.last(), Some(GuiCommand::SelectCamera { id }) if id == "cam-b"),
                "{sent:?}"
            );
        });
    }

    #[test]
    fn picking_a_camera_with_no_known_ids_sends_nothing() {
        run(|| {
            let (tx, mut rx) = channel_pair();
            let (dropdown, _handler) = build_camera_dropdown(&tx, Rc::default());
            dropdown.set_model(Some(&gtk::StringList::new(&["A", "B"])));
            dropdown.set_selected(1);
            assert!(drain(&mut rx).is_empty());
        });
    }

    #[test]
    fn the_live_pill_starts_hidden_with_a_dot_then_its_text() {
        run(|| {
            let (pill, label) = build_live_pill();
            assert!(!pill.is_visible());
            assert!(pill.has_css_class("oe-pill"));
            let parts = children(&pill);
            assert_eq!(type_names(&parts), ["GtkBox", "GtkLabel"]);
            assert!(parts[0].has_css_class("oe-live-dot"));
            assert_eq!(parts[1], label.clone().upcast::<gtk::Widget>());
            assert_eq!(label.label(), "Virtual camera live");
        });
    }

    #[test]
    fn the_compare_button_names_itself_and_hides_its_icon() {
        run(|| {
            let button = build_compare_button();
            assert!(button.has_css_class("oe-pill-button"));
            assert_eq!(
                button.tooltip_text().as_deref(),
                Some("View original camera feed")
            );
            let content = button.child().unwrap();
            let parts = children(&content);
            assert_eq!(type_names(&parts), ["GtkImage", "GtkLabel"]);
            assert!(gtk::test_accessible_has_role(
                &parts[0],
                gtk::AccessibleRole::Presentation
            ));
            let text = parts[1].downcast_ref::<gtk::Label>().unwrap();
            assert_eq!(text.label(), "Hold to Compare");
        });
    }

    #[test]
    fn the_node_pill_shows_the_virtual_camera_name() {
        run(|| {
            let pill = build_node_pill();
            assert_eq!(pill.label(), VCAM_NODE_NAME);
            assert!(pill.has_css_class("oe-pill") && pill.has_css_class("monospace"));
            assert!(pill.tooltip_text().unwrap().contains("Zoom"));
        });
    }

    #[test]
    fn the_frame_stacks_the_picture_then_each_overlay_in_order() {
        run(|| {
            let (tx, _rx) = channel_pair();
            let preview = build(&tx, Rc::default());
            let layers = children(&preview.frame);
            assert_eq!(
                type_names(&layers),
                [
                    "GtkPicture",
                    "GtkLabel",
                    "GtkBox",
                    "GtkDropDown",
                    "GtkBox",
                    "GtkButton",
                    "GtkLabel"
                ]
            );
            assert_eq!(
                layers[3],
                preview.camera_dropdown.clone().upcast::<gtk::Widget>()
            );
            assert_eq!(layers[5], preview.compare.clone().upcast::<gtk::Widget>());
            assert_eq!(layers[6], preview.node_pill.clone().upcast::<gtk::Widget>());
            assert!(preview.frame.has_css_class("oe-preview-frame"));
        });
    }
}
