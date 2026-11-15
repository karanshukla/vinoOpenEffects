mod args;
mod bus;
mod daemon;
mod effects;
mod values;

use clap::Parser;
use zbus::Connection;

use args::{Cli, Command, EnableFlags};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    match Cli::parse().command {
        Command::Autostart { command } => daemon::autostart(&command),
        Command::Doctor { json } => daemon::doctor(json).await,
        command => {
            let conn = Connection::session().await?;
            bus::ensure_daemon(&conn).await;
            run_on_daemon(&conn, command).await
        }
    }
}

async fn run_on_daemon(conn: &Connection, command: Command) -> anyhow::Result<()> {
    match command {
        Command::Status { json, short } => daemon::status(conn, json, short).await,
        Command::Inference {
            provider,
            retry,
            json,
        } => daemon::inference(conn, provider, retry, json).await,
        Command::Start => daemon::start(conn).await,
        Command::Stop => daemon::stop(conn).await,
        Command::Camera { command } => daemon::camera(conn, command).await,

        Command::Effects => effects::list(conn).await,
        Command::Enable { effect } => effects::enable(conn, effect, true).await,
        Command::Disable { effect } => effects::enable(conn, effect, false).await,
        Command::Toggle { effect } => effects::toggle(conn, effect).await,
        Command::CenterStage(args) => effects::center_stage(conn, args).await,
        Command::PortraitBlur { toggle, strength } => {
            effects::portrait_blur(conn, toggle, strength).await
        }
        Command::Background { value, off } => effects::background(conn, value, off).await,
        Command::StudioLight(args) => effects::studio_light(conn, args).await,
        Command::Reactions { toggle } => effects::reactions(conn, toggle).await,
        Command::React { reaction } => effects::react(conn, reaction).await,
        Command::Passthrough { on, off } => {
            effects::passthrough(conn, EnableFlags { on, off }.resolve()).await
        }
        Command::Set { assignment, value } => {
            effects::set_raw_param(conn, &assignment, &value).await
        }
        Command::Watch => effects::watch(conn).await,

        Command::Autostart { .. } | Command::Doctor { .. } => {
            unreachable!("handled before the daemon connection")
        }
    }
}
