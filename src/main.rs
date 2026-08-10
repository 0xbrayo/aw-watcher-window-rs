mod afk;
mod client;
mod config;
mod privacy;
mod window;

use afk::{AFK_CLIENT_NAME, resolve_afk_status};
use config::{Args, CLIENT_NAME};
use privacy::TitleFilter;
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::load();

    init_logging(args.verbose);

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("aw-watcher-window-rs currently only supports macOS");
        std::process::exit(1);
    }

    #[cfg(target_os = "macos")]
    {
        run(args).await
    }
}

fn init_logging(verbose: bool) {
    let default_level = if verbose { "debug" } else { "info" };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

#[cfg(target_os = "macos")]
async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    window::ensure_accessibility_warning();

    let filter = match TitleFilter::new(args.exclude_title, &args.exclude_titles) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    let hostname = gethostname::gethostname().to_string_lossy().into_owned();
    let port = args.server_port();
    let pulsetime = args.pulsetime();
    let poll = std::time::Duration::from_secs_f64(args.poll_time.max(0.1));
    let afk_timeout = args.afk_timeout.max(1.0);

    let client = client::AwClient::new(&args.host, port, hostname.clone());

    let window_bucket = client::AwClient::bucket_id_for(CLIENT_NAME, &hostname);
    let afk_bucket = client::AwClient::bucket_id_for(AFK_CLIENT_NAME, &hostname);

    info!(
        "Starting {} → {} | window bucket={} type={} | afk bucket={} type={} (timeout={:.0}s, enabled={}) | poll={:.1}s pulsetime={:.1}s",
        CLIENT_NAME,
        client.base_url(),
        window_bucket,
        args.event_type,
        afk_bucket,
        args.afk_event_type,
        afk_timeout,
        !args.disable_afk,
        args.poll_time,
        pulsetime,
    );

    client
        .wait_and_create_bucket(&window_bucket, CLIENT_NAME, &args.event_type)
        .await?;

    if !args.disable_afk {
        client
            .wait_and_create_bucket(&afk_bucket, AFK_CLIENT_NAME, &args.afk_event_type)
            .await?;
        if args.afk_event_type != "afkstatus" {
            info!(
                "AFK event type is {:?} (official uses afkstatus). Login screen → afk; idle ≥ {:.0}s → afk.",
                args.afk_event_type, afk_timeout
            );
        }
    }

    let exit_with_parent = args.exit_with_parent;
    let initial_ppid = current_ppid();
    if exit_with_parent {
        info!("--exit-with-parent enabled (initial ppid={})", initial_ppid);
    }

    loop {
        if exit_with_parent && current_ppid() == 1 && initial_ppid != 1 {
            info!("Parent process died; exiting");
            break;
        }

        window::pump_runloop();

        let window_data = window::get_active_window().map(|d| filter.apply(d));

        if let Some(ref data) = window_data {
            debug!("window: {:?}", data);
            client
                .heartbeat(&window_bucket, data.clone(), pulsetime)
                .await;
        } else {
            debug!("Unable to fetch window; trying again on next poll");
        }

        if !args.disable_afk {
            let afk = resolve_afk_status(window_data.as_ref(), afk_timeout);
            debug!("afk: {:?}", afk);
            client.heartbeat(&afk_bucket, afk, pulsetime).await;
        }

        tokio::time::sleep(poll).await;
    }

    Ok(())
}

fn current_ppid() -> i32 {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn getppid() -> i32;
        }
        unsafe { getppid() }
    }
    #[cfg(not(unix))]
    {
        0
    }
}
