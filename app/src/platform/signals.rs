//! Turning SIGINT and SIGTERM into a clean quit, so the log is flushed and a share is not left
//! running on the server when the session ends.

use async_channel::Sender;
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

/// Returns a receiver that yields once per termination signal. A detached thread blocks waiting
/// for them (it costs nothing while idle); handling stays on the main loop.
pub fn watch() -> Option<async_channel::Receiver<()>> {
    let (tx, rx): (Sender<()>, _) = async_channel::unbounded();
    let mut signals = match Signals::new([SIGINT, SIGTERM]) {
        Ok(signals) => signals,
        Err(err) => {
            tracing::warn!(%err, "couldn't watch for termination signals");
            return None;
        }
    };
    std::thread::Builder::new()
        .name("signals".into())
        .spawn(move || {
            for _ in signals.forever() {
                let _ = tx.try_send(());
            }
        })
        .ok()?;
    Some(rx)
}
