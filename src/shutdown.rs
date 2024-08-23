/// Registers signal handlers and waits for a signal that
/// indicates a shutdown request.
pub async fn wait_for_signal() {
    wait_for_signal_impl().await
}

/// Waits for a signal that requests a graceful shutdown, like SIGTERM or SIGINT.
#[cfg(unix)]
async fn wait_for_signal_impl() {
    use tokio::signal::unix::{signal, SignalKind};

    // Infos here:
    // https://www.gnu.org/software/libc/manual/html_node/Termination-Signals.html
    let mut signal_terminate = signal(SignalKind::terminate()).unwrap();
    let mut signal_interrupt = signal(SignalKind::interrupt()).unwrap();

    tokio::select! {
        _ = signal_terminate.recv() => log::info!("Received SIGTERM."),
        _ = signal_interrupt.recv() => log::info!("Received SIGINT."),
    };
}

/// Waits for a signal that requests a graceful shutdown, Ctrl-C (SIGINT).
#[cfg(windows)]
async fn wait_for_signal_impl() {
    use tokio::signal::windows;

    // Infos here:
    // https://learn.microsoft.com/en-us/windows/console/handlerroutine
    let mut signal_c = windows::ctrl_c().unwrap();
    let mut signal_break = windows::ctrl_break().unwrap();
    let mut signal_close = windows::ctrl_close().unwrap();
    let mut signal_shutdown = windows::ctrl_shutdown().unwrap();

    tokio::select! {
        _ = signal_c.recv() => log::info!("Received CTRL_C."),
        _ = signal_break.recv() => log::info!("Received CTRL_BREAK."),
        _ = signal_close.recv() => log::info!("Received CTRL_CLOSE."),
        _ = signal_shutdown.recv() => log::info!("Received CTRL_SHUTDOWN."),
    };
}
