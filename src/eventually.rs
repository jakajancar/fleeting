use std::{fmt::Display, future::Future, result::Result, time::SystemTime};
use tokio::time::{sleep, timeout, Duration};

pub const SECOND: Duration = Duration::from_secs(1);
pub const MINUTE: Duration = Duration::from_secs(60);

pub enum EventuallyResult<T, E> {
    Ok(T),
    TempErr(E),
    PermErr(E),
}

// output errors with debug error level, final one returns normally as Err
pub async fn eventually<A, AFut, T, E>(sleep_between: Duration, total_timeout: Duration, mut attempt: A) -> Result<T, E>
where
    A: FnMut() -> AFut,
    AFut: Future<Output = EventuallyResult<T, E>>,
    E: Display,
{
    let deadline = SystemTime::now() + total_timeout;
    let mut attempt_no = 0usize;
    loop {
        attempt_no += 1;

        match attempt().await {
            EventuallyResult::Ok(t) => break Ok(t),
            EventuallyResult::PermErr(e) => break Err(e),
            EventuallyResult::TempErr(e) => {
                log::debug!("Attempt {attempt_no} failed: {e:#}");
                if SystemTime::now() + sleep_between > deadline {
                    log::error!("Giving up");
                    break Err(e);
                } else {
                    sleep(sleep_between).await;
                }
            }
        }
    }
}
