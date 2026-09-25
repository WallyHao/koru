//! Terminal-owned approval for one immutable prepared effect.
use crate::{
    error::{ErrorCode, KoruError, Result},
    permissions::{Decision, PreparedAction},
};
use std::{
    io::{self, BufRead, IsTerminal, Read, Write},
    sync::mpsc,
    thread,
    time::Instant,
};

/// Ask for one decision. A redirected session never waits for input.
pub fn approve(action: &PreparedAction, deadline: Instant) -> Result<Decision> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(noninteractive());
    }
    let mut output = io::stderr().lock();
    preview(action, &mut output)?;
    read_before_deadline(deadline, move || {
        let mut answer = String::new();
        io::stdin().lock().take(16).read_line(&mut answer)?;
        Ok(answer)
    })
}

/// Injectable approval boundary used by terminal fixtures.
pub fn decide(
    action: &PreparedAction,
    input: &mut impl BufRead,
    output: &mut impl Write,
    interactive: bool,
    deadline: Instant,
) -> Result<Decision> {
    if !interactive {
        return Err(noninteractive());
    }
    if Instant::now() >= deadline {
        return Err(KoruError::new(
            ErrorCode::Timeout,
            "approval deadline exceeded",
        ));
    }
    preview(action, output)?;
    let mut answer = String::new();
    Read::take(input, 16)
        .read_line(&mut answer)
        .map_err(|error| KoruError::io("cannot read approval", error))?;
    if Instant::now() >= deadline {
        return Err(timeout());
    }
    Ok(parse_answer(&answer))
}

fn preview(action: &PreparedAction, output: &mut impl Write) -> Result<()> {
    writeln!(
        output,
        "Approve action {} for {}?\n{}\nType yes to approve: ",
        action.id(),
        action.command(),
        action.display_preview()
    )
    .map_err(|error| KoruError::io("cannot display approval", error))?;
    output
        .flush()
        .map_err(|error| KoruError::io("cannot flush approval", error))?;
    Ok(())
}

fn parse_answer(answer: &str) -> Decision {
    if answer.trim_end_matches(['\r', '\n']) == "yes" {
        Decision::ApproveOnce
    } else {
        Decision::Deny
    }
}

fn read_before_deadline(
    deadline: Instant,
    read: impl FnOnce() -> io::Result<String> + Send + 'static,
) -> Result<Decision> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(timeout)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = sender.send(read());
    });
    let answer = receiver
        .recv_timeout(remaining)
        .map_err(|_| timeout())?
        .map_err(|error| KoruError::io("cannot read approval", error))?;
    Ok(parse_answer(&answer))
}

fn noninteractive() -> KoruError {
    KoruError::new(
        ErrorCode::PermissionDenied,
        "confirmation requires an interactive terminal; run the command in a terminal",
    )
}

fn timeout() -> KoruError {
    KoruError::new(ErrorCode::Timeout, "approval deadline exceeded")
}

#[cfg(test)]
mod tests {
    use super::read_before_deadline;
    use crate::error::ErrorCode;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };

    #[test]
    fn blocked_approval_input_obeys_the_deadline() {
        let (sender, receiver) = mpsc::channel::<()>();
        let started = Instant::now();
        let error = read_before_deadline(started + Duration::from_millis(20), move || {
            let _ = receiver.recv();
            Ok("yes\n".to_owned())
        })
        .unwrap_err();
        drop(sender);
        assert_eq!(error.code(), ErrorCode::Timeout);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
