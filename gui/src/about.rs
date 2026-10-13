//! Native `AdwAboutDialog`, opened from the primary menu.

use adw::prelude::*;

use crate::constants::APP_ID;

pub fn present(parent: &impl IsA<gtk::Widget>) {
    let about = adw::AboutDialog::builder()
        .application_name("vinoOpenEffects")
        .application_icon(APP_ID)
        .version(env!("VINOOPENEFFECTS_VERSION"))
        .comments("Linux-native webcam effects engine")
        .developer_name("Karan Shukla")
        .developers(["Karan Shukla https://github.com/karanshukla"])
        .copyright("© 2026 Karan Shukla\nBased on openeffects © Aryan Kushwaha and contributors")
        .license_type(gtk::License::Gpl30)
        .website("https://github.com/karanshukla/vinoOpenEffects")
        .issue_url("https://github.com/karanshukla/vinoOpenEffects/issues")
        .build();

    about.add_credit_section(
        Some("Original author of openeffects"),
        &["Aryan Kushwaha https://github.com/funinkina"],
    );
    about.add_link(
        "openeffects (upstream)",
        "https://github.com/funinkina/openeffects",
    );

    about.present(Some(parent));
}
