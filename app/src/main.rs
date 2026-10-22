mod application;
mod controller;
mod crash_reporting;
mod host_env;
mod i18n;
mod log_privacy;
mod platform;
mod ui;

use gtk::prelude::*;

fn main() -> glib::ExitCode {
    // Checked before *everything* else, including `--version`: a real user never passes this
    // flag; it exists only so this same binary can re-exec itself as the crash-dump IPC server
    // (see `crash_reporting`'s module doc comment). The spawned server must never read settings,
    // open the network or initialise GTK, so this has to come first.
    if let Some(socket_name) = crash_reporting::crash_server_socket_name() {
        let paths = jastrzab_core::AppPaths::resolve().expect("resolve XDG application directories");
        crash_reporting::run_crash_server(&socket_name, paths.logs_dir(), paths.crash_dumps_dir());
    }

    // `--version` / `-v`: print and exit before any side effect, notably before the state
    // directories are created. Parsed by hand because GApplication's option handling only runs
    // inside `app.run()`.
    if std::env::args().any(|arg| arg == "--version" || arg == "-v") {
        println!("{}", application::version_line());
        return glib::ExitCode::SUCCESS;
    }

    // A packaged launcher can run this binary under another name (an AppImage's is
    // `AppRun.wrapped`); GTK and AT-SPI take the program name from argv[0], so pin it.
    glib::set_prgname(Some("jastrzab"));

    // Resolved here, synchronously: logging and the panic hook below need `logs_dir()` before
    // anything else runs, so they also catch failures during the rest of start-up.
    let Some(paths) = jastrzab_core::AppPaths::resolve() else {
        eprintln!("jastrzab: couldn't find the home folder (is HOME set?)");
        return glib::ExitCode::FAILURE;
    };
    let early_dirs = paths.ensure_early_dirs();

    let log_handle = crash_reporting::init_logging(&paths);
    // One of the very first lines in every log, on purpose: a pasted log snippet is
    // self-identifying (which build, which OS and architecture, where the rest of the log lives)
    // without anyone having to ask.
    tracing::info!(
        version = %application::version_line(),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        state_dir = %paths.state_dir().display(),
        "starting up"
    );
    if let Err(err) = &early_dirs {
        tracing::warn!(%err, "couldn't create the log and crash-dump folders");
    }
    crash_reporting::install_panic_hook(log_handle.clone());
    // Failure here degrades to a warning inside `attach_crash_handler` itself: crash-dump capture
    // must never block the app from starting.
    let _crash_client = crash_reporting::attach_crash_handler();

    let app = application::build_application(paths);
    let exit_code = app.run();
    log_handle.flush();
    exit_code
}
