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
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(timeout)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut answer = String::new();
        let outcome = io::stdin().lock().take(16).read_line(&mut answer);
        let _ = sender.send(outcome.map(|_| answer));
    });
    let answer = receiver
        .recv_timeout(remaining)
        .map_err(|_| timeout())?
        .map_err(|error| KoruError::io("cannot read approval", error))?;
    Ok(parse_answer(&answer))
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

fn noninteractive() -> KoruError {
    KoruError::new(
        ErrorCode::PermissionDenied,
        "confirmation requires an interactive terminal; run the command in a terminal",
    )
}

fn timeout() -> KoruError {
    KoruError::new(ErrorCode::Timeout, "approval deadline exceeded")
}
