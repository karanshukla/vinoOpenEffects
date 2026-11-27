//! Shared building blocks for the effect cards: the expandable
//! [`EffectCard`] itself, plus segmented-control ([`ToggleCtl`]) and slider
//! ([`SliderCtl`]) rows that the per-effect modules own directly.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::glib;
use adw::prelude::*;

use crate::dbus_client::{CmdTx, GuiCommand};

/// Trailing debounce for slider drags: one `SetParam` per pause, since each
/// makes the daemon save, reapply the pipeline and broadcast.
const SLIDER_DEBOUNCE: Duration = Duration::from_millis(120);
/// How long after the last move daemon echoes are ignored, so a stale echo
/// can't snap a slider back mid-drag.
const SLIDER_ECHO_HOLDOFF: Duration = Duration::from_millis(1000);

/// An `AdwToggleGroup` (from [`toggle_row`]) forwarding its selection.
/// [`ToggleCtl::set_value`] blocks the handler, so state sync doesn't echo.
pub struct ToggleCtl {
    pub group: adw::ToggleGroup,
    handler: glib::SignalHandlerId,
}

impl ToggleCtl {
    /// Select the toggle whose name equals `value` (no-op if none matches).
    pub fn set_value(&self, value: &str) {
        self.group.block_signal(&self.handler);
        self.group.set_active_name(Some(value));
        self.group.unblock_signal(&self.handler);
    }
}

/// A settings row with a compact segmented control on the right;
/// `on_change` gets the chosen toggle's name.
pub fn toggle_row(
    title: &str,
    subtitle: &str,
    options: &[(&'static str, &'static str)],
    on_change: impl Fn(&str) + 'static,
) -> (adw::ActionRow, ToggleCtl) {
    let group = adw::ToggleGroup::builder()
        .valign(gtk::Align::Center)
        .css_classes(["round"])
        .build();
    group.update_property(&[gtk::accessible::Property::Label(title)]);
    for (name, label) in options {
        group.add(adw::Toggle::builder().name(*name).label(*label).build());
    }
    let handler = group.connect_active_notify(move |group| {
        if let Some(name) = group.active_name() {
            on_change(name.as_str());
        }
    });
    let ctl = ToggleCtl { group, handler };

    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    row.add_suffix(&ctl.group);
    (row, ctl)
}

/// A slider row. [`SliderCtl::set_value`] blocks its handler, so state sync
/// doesn't echo.
pub struct SliderCtl {
    adjustment: gtk::Adjustment,
    scale: gtk::Scale,
    spin: gtk::SpinButton,
    handler: glib::SignalHandlerId,
    /// When the user last moved this slider (drives the echo holdoff).
    last_input: Rc<Cell<Option<Instant>>>,
}

impl SliderCtl {
    pub fn set_value(&self, value: f64) {
        if let Some(at) = self.last_input.get()
            && at.elapsed() < SLIDER_ECHO_HOLDOFF
        {
            return;
        }
        if (self.adjustment.value() - value).abs() < 0.5 {
            return;
        }
        self.adjustment.block_signal(&self.handler);
        self.adjustment.set_value(value);
        self.adjustment.unblock_signal(&self.handler);
    }

    /// Replace the screen-reader name (the visible title by default), for a
    /// title that repeats elsewhere in the card, e.g. a second "Brightness".
    pub fn set_accessible_name(&self, name: &str) {
        for widget in [
            self.scale.upcast_ref::<gtk::Widget>(),
            self.spin.upcast_ref(),
        ] {
            widget.reset_relation(gtk::AccessibleRelation::LabelledBy);
            widget.update_property(&[gtk::accessible::Property::Label(name)]);
        }
    }
}

/// A slider row: title and spin button, then a scale sharing its adjustment.
/// Bipolar ranges get a tick at 0. Only the last value of a burst is sent.
#[allow(clippy::too_many_arguments)]
fn slider_row(
    title: &str,
    subtitle: &str,
    min: f64,
    max: f64,
    step: f64,
    id: &'static str,
    key: &'static str,
    cmd_tx: &CmdTx,
    to_value: impl Fn(f64) -> zvariant::OwnedValue + 'static,
) -> (gtk::ListBoxRow, SliderCtl) {
    let adjustment = gtk::Adjustment::new(min, min, max, step, step * 10.0, 0.0);

    let (body, scale, spin) = build_slider_body(title, subtitle, &adjustment, min, max, step);
    let (handler, last_input) = connect_debounced_param(&adjustment, id, key, cmd_tx, to_value);

    (
        plain_row(&body),
        SliderCtl {
            adjustment,
            scale,
            spin,
            handler,
            last_input,
        },
    )
}

fn build_slider_body(
    title: &str,
    subtitle: &str,
    adjustment: &gtk::Adjustment,
    min: f64,
    max: f64,
    step: f64,
) -> (gtk::Box, gtk::Scale, gtk::SpinButton) {
    let title_label = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .hexpand(true)
        .build();
    let spin = gtk::SpinButton::builder()
        .adjustment(adjustment)
        .climb_rate(step)
        .digits(0)
        .numeric(true)
        .valign(gtk::Align::Center)
        .build();
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    header.append(&title_label);
    header.append(&spin);

    let scale = gtk::Scale::builder()
        .orientation(gtk::Orientation::Horizontal)
        .adjustment(adjustment)
        .digits(0)
        .draw_value(false)
        .hexpand(true)
        .build();
    if min < 0.0 && max > 0.0 {
        scale.add_mark(0.0, gtk::PositionType::Bottom, None);
    }

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();
    body.append(&header);
    for widget in [scale.upcast_ref::<gtk::Widget>(), spin.upcast_ref()] {
        widget.update_relation(&[gtk::accessible::Relation::LabelledBy(&[
            title_label.upcast_ref()
        ])]);
    }
    if !subtitle.is_empty() {
        let sub = gtk::Label::builder()
            .label(subtitle)
            .xalign(0.0)
            .wrap(true)
            .css_classes(["caption", "dim-label"])
            .build();
        for widget in [scale.upcast_ref::<gtk::Widget>(), spin.upcast_ref()] {
            widget.update_relation(&[gtk::accessible::Relation::DescribedBy(&[sub.upcast_ref()])]);
        }
        body.append(&sub);
    }
    body.append(&scale);

    (body, scale, spin)
}

fn connect_debounced_param(
    adjustment: &gtk::Adjustment,
    id: &'static str,
    key: &'static str,
    cmd_tx: &CmdTx,
    to_value: impl Fn(f64) -> zvariant::OwnedValue + 'static,
) -> (glib::SignalHandlerId, Rc<Cell<Option<Instant>>>) {
    let cmd_tx = cmd_tx.clone();
    let last_input: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
    let pending: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
    let handler = {
        let last_input = last_input.clone();
        adjustment.connect_value_changed(move |adj| {
            last_input.set(Some(Instant::now()));
            if let Some(source) = pending.take() {
                source.remove();
            }
            let value = to_value(adj.value());
            let cmd_tx = cmd_tx.clone();
            let pending_done = pending.clone();
            pending.set(Some(glib::timeout_add_local_once(
                SLIDER_DEBOUNCE,
                move || {
                    pending_done.set(None);
                    let _ = cmd_tx.send(GuiCommand::SetParam {
                        id: id.to_string(),
                        key: key.to_string(),
                        value,
                    });
                },
            )));
        })
    };
    (handler, last_input)
}

/// A slider row reporting an unsigned integer `SetParam`.
#[allow(clippy::too_many_arguments)]
pub fn slider_row_u32(
    title: &str,
    subtitle: &str,
    min: f64,
    max: f64,
    step: f64,
    id: &'static str,
    key: &'static str,
    cmd_tx: &CmdTx,
) -> (gtk::ListBoxRow, SliderCtl) {
    slider_row(title, subtitle, min, max, step, id, key, cmd_tx, |v| {
        shared::dbus::u32_value(v as u32)
    })
}

/// A slider row reporting a signed integer `SetParam`.
#[allow(clippy::too_many_arguments)]
pub fn slider_row_i32(
    title: &str,
    subtitle: &str,
    min: f64,
    max: f64,
    step: f64,
    id: &'static str,
    key: &'static str,
    cmd_tx: &CmdTx,
) -> (gtk::ListBoxRow, SliderCtl) {
    slider_row(title, subtitle, min, max, step, id, key, cmd_tx, |v| {
        shared::dbus::i32_value(v as i32)
    })
}

/// Wrap `child` in a padded, non-activatable list row, for custom content
/// (sliders, thumbnail grids) inside an effect card.
pub fn plain_row(child: &impl IsA<gtk::Widget>) -> gtk::ListBoxRow {
    child.set_margin_top(12);
    child.set_margin_bottom(12);
    child.set_margin_start(12);
    child.set_margin_end(12);
    gtk::ListBoxRow::builder()
        .activatable(false)
        .selectable(false)
        .child(child)
        .build()
}

/// A small heading that splits an effect card into sections ("On You",
/// "Behind You", "Images", ...).
pub fn section_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["caption-heading", "dim-label"])
        .accessible_role(gtk::AccessibleRole::Heading)
        .build()
}

/// One effect's card: icon, name, summary and switch, expanding to its
/// settings. [`EffectCard::set_on`] blocks the switch handler, so state sync
/// doesn't echo.
pub struct EffectCard {
    /// The `boxed-list` that draws the card; add this to the column.
    pub list: gtk::ListBox,
    expander: adw::ExpanderRow,
    switch: gtk::Switch,
    handler: glib::SignalHandlerId,
    icon: gtk::Image,
}

impl EffectCard {
    /// `on_toggle` receives the new switch state whenever the user flips it.
    pub fn new(title: &str, icon_name: &str, on_toggle: impl Fn(bool) + 'static) -> Self {
        let icon = gtk::Image::builder()
            .icon_name(icon_name)
            .pixel_size(20)
            .valign(gtk::Align::Center)
            .css_classes(["oe-effect-icon"])
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        let switch = gtk::Switch::builder()
            .valign(gtk::Align::Center)
            .tooltip_text(title)
            .build();
        switch.update_property(&[gtk::accessible::Property::Label(title)]);
        let handler = switch.connect_active_notify(move |sw| on_toggle(sw.is_active()));

        let expander = adw::ExpanderRow::builder()
            .title(title)
            .subtitle("Off")
            .build();
        expander.add_prefix(&icon);
        expander.add_suffix(&switch);

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        list.append(&expander);

        Self {
            list,
            expander,
            switch,
            handler,
            icon,
        }
    }

    /// Reflect the effect's enabled state without echoing it to the daemon.
    pub fn set_on(&self, on: bool) {
        self.switch.block_signal(&self.handler);
        self.switch.set_active(on);
        self.switch.unblock_signal(&self.handler);
        if on {
            self.icon.add_css_class("oe-on");
        } else {
            self.icon.remove_css_class("oe-on");
        }
    }

    pub fn is_on(&self) -> bool {
        self.switch.is_active()
    }

    /// The one-line summary under the effect name ("Normal zoom · one face").
    pub fn set_summary(&self, summary: &str) {
        self.expander.set_subtitle(summary);
    }

    pub fn set_expanded(&self, expanded: bool) {
        self.expander.set_expanded(expanded);
    }

    pub fn add_row(&self, row: &impl IsA<gtk::Widget>) {
        self.expander.add_row(row);
    }
}

pub fn pref_group(title: &str, description: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(title).build();
    if !description.is_empty() {
        group.set_description(Some(description));
    }
    group
}

#[cfg(test)]
mod widget_tests {
    use super::*;
    use crate::gtk_test::{channel_pair, children, drain, pump, run, type_names};

    fn body(subtitle: &str, min: f64, max: f64) -> (gtk::Box, gtk::Scale, gtk::SpinButton) {
        let adjustment = gtk::Adjustment::new(min, min, max, 1.0, 10.0, 0.0);
        build_slider_body("Intensity", subtitle, &adjustment, min, max, 1.0)
    }

    #[test]
    fn a_slider_without_a_subtitle_is_a_title_line_then_the_scale() {
        run(|| {
            let (body, scale, spin) = body("", 0.0, 100.0);
            let parts = children(&body);
            assert_eq!(type_names(&parts), ["GtkBox", "GtkScale"]);
            assert_eq!(parts[1], scale.clone().upcast::<gtk::Widget>());
            let header = children(&parts[0]);
            assert_eq!(type_names(&header), ["GtkLabel", "GtkSpinButton"]);
            let title = header[0].downcast_ref::<gtk::Label>().unwrap();
            assert_eq!(title.label(), "Intensity");
            assert_eq!(header[1], spin.upcast::<gtk::Widget>());
            assert_eq!(body.orientation(), gtk::Orientation::Vertical);
        });
    }

    #[test]
    fn a_slider_subtitle_sits_between_the_title_line_and_the_scale() {
        run(|| {
            let (body, scale, _spin) = body("Darken the room", 0.0, 100.0);
            let parts = children(&body);
            assert_eq!(type_names(&parts), ["GtkBox", "GtkLabel", "GtkScale"]);
            let sub = parts[1].downcast_ref::<gtk::Label>().unwrap();
            assert_eq!(sub.label(), "Darken the room");
            assert!(sub.has_css_class("dim-label") && sub.has_css_class("caption"));
            assert!(gtk::test_accessible_has_relation(
                &scale,
                gtk::AccessibleRelation::DescribedBy
            ));
        });
    }

    #[test]
    fn the_scale_and_spin_button_are_named_by_the_title() {
        run(|| {
            let (_body, scale, spin) = body("", 0.0, 100.0);
            for widget in [scale.upcast_ref::<gtk::Widget>(), spin.upcast_ref()] {
                assert!(gtk::test_accessible_has_relation(
                    widget,
                    gtk::AccessibleRelation::LabelledBy
                ));
            }
        });
    }

    #[test]
    fn the_scale_and_spin_button_share_one_adjustment() {
        run(|| {
            let adjustment = gtk::Adjustment::new(0.0, 0.0, 100.0, 1.0, 10.0, 0.0);
            let (_body, scale, spin) =
                build_slider_body("Contrast", "", &adjustment, 0.0, 100.0, 1.0);
            adjustment.set_value(42.0);
            assert_eq!(scale.value(), 42.0);
            assert_eq!(spin.value(), 42.0);
            assert_eq!(spin.digits(), 0);
            assert!(spin.is_numeric());
            assert!(!scale.draws_value());
        });
    }

    #[test]
    fn a_burst_of_slider_moves_sends_one_set_param_with_the_last_value() {
        run(|| {
            let (tx, mut rx) = channel_pair();
            let (_row, ctl) = slider_row_u32(
                "Intensity",
                "",
                0.0,
                100.0,
                1.0,
                "studio_light",
                "intensity",
                &tx,
            );
            for v in [10.0, 20.0, 30.0] {
                ctl.adjustment.set_value(v);
            }
            assert!(drain(&mut rx).is_empty(), "sent before the debounce");
            pump(400);
            let sent = drain(&mut rx);
            assert_eq!(sent.len(), 1, "{sent:?}");
            match &sent[0] {
                GuiCommand::SetParam { id, key, value } => {
                    assert_eq!((id.as_str(), key.as_str()), ("studio_light", "intensity"));
                    assert_eq!(shared::dbus::value_as_u32(value), Some(30));
                }
                other => panic!("unexpected {other:?}"),
            }
        });
    }

    #[test]
    fn a_signed_slider_sends_a_negative_value() {
        run(|| {
            let (tx, mut rx) = channel_pair();
            let (_row, ctl) = slider_row_i32(
                "Brightness",
                "",
                -100.0,
                100.0,
                1.0,
                "studio_light",
                "brightness",
                &tx,
            );
            ctl.adjustment.set_value(-25.0);
            pump(400);
            let sent = drain(&mut rx);
            match sent.as_slice() {
                [GuiCommand::SetParam { value, .. }] => {
                    assert_eq!(shared::dbus::value_as_i32(value), Some(-25));
                }
                other => panic!("unexpected {other:?}"),
            }
        });
    }

    #[test]
    fn setting_a_slider_from_the_daemon_does_not_send_it_back() {
        run(|| {
            let (tx, mut rx) = channel_pair();
            let (_row, ctl) = slider_row_u32(
                "Intensity",
                "",
                0.0,
                100.0,
                1.0,
                "studio_light",
                "intensity",
                &tx,
            );
            ctl.set_value(60.0);
            pump(300);
            assert_eq!(ctl.adjustment.value(), 60.0);
            assert!(drain(&mut rx).is_empty());
        });
    }
}
