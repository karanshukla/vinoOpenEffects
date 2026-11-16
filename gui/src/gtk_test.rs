//! GTK runs on one thread per process, but the test harness gives every test
//! its own. `run` sends a test body to a single long-lived GTK thread, so
//! widget tests can be separate `#[test]`s. Without a display the body is
//! skipped, unless `VINOOPENEFFECTS_REQUIRE_DISPLAY` is set (CI sets it).

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};

type Job = Box<dyn FnOnce(bool) + Send>;

fn worker() -> &'static Mutex<Sender<Job>> {
    static WORKER: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::spawn(move || {
            let ready = crate::a11y_tests::init_gtk();
            for job in rx {
                job(ready);
            }
        });
        Mutex::new(tx)
    })
}

/// Run `body` on the GTK thread and re-raise its panic here.
pub fn run(body: impl FnOnce() + Send + 'static) {
    let (done_tx, done_rx) = channel();
    let job: Job = Box::new(move |ready| {
        let result = if ready {
            catch_unwind(AssertUnwindSafe(body))
        } else {
            Ok(())
        };
        let _ = done_tx.send(result);
    });
    worker()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .send(job)
        .expect("GTK thread is gone");
    if let Err(panic) = done_rx.recv().expect("GTK thread dropped the job") {
        resume_unwind(panic);
    }
}

use gtk::prelude::*;

use crate::dbus_client::{CmdTx, GuiCommand};

pub type CmdRx = tokio::sync::mpsc::UnboundedReceiver<GuiCommand>;

pub fn channel_pair() -> (CmdTx, CmdRx) {
    tokio::sync::mpsc::unbounded_channel()
}

/// Everything sent so far.
pub fn drain(rx: &mut CmdRx) -> Vec<GuiCommand> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

pub fn children(widget: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut out = Vec::new();
    let mut child = widget.first_child();
    while let Some(c) = child {
        child = c.next_sibling();
        out.push(c);
    }
    out
}

pub fn type_names(widgets: &[gtk::Widget]) -> Vec<String> {
    widgets
        .iter()
        .map(|w| w.type_().name().to_string())
        .collect()
}

/// Run the main loop for `ms` milliseconds so timers fire.
pub fn pump(ms: u64) {
    let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    let context = gtk::glib::MainContext::default();
    while std::time::Instant::now() < end {
        context.iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
