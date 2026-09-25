//! Interactive task input and one-use approval decisions.
//!
//! Both prompts require an interactive terminal on standard input and standard
//! error. The task prompt is enabled for UTF-8 erase handling so that deleting
//! multi-byte characters behaves like deleting ASCII; approval reads one short
//! answer line and treats anything other than `y`/`yes` as a denial.
use super::style::Style;
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

const MAX_TASK_BYTES: usize = 8 * 1024;
const MAX_ANSWER_BYTES: u64 = 16;

/// Read one bounded shell task from an interactive terminal.
pub fn read_task() -> Result<String> {
    if !interactive() {
        return Err(KoruError::new(
            ErrorCode::Validation,
            "shell needs a task argument when input is redirected",
        ));
    }
    let stdin = io::stdin();
    let style = Style::stderr();
    let mut output = io::stderr().lock();
    write!(output, "{}", style.bold("Enter the command description:"))
        .and_then(|()| writeln!(output))
        .and_then(|()| write!(output, "{} ", style.cyan(">")))
        .and_then(|()| output.flush())
        .map_err(|error| KoruError::io("cannot display task prompt", error))?;
    #[cfg(unix)]
    {
        with_utf8_erase(&stdin, || read_task_line(&mut stdin.lock()))
    }
    #[cfg(not(unix))]
    {
        read_task_line(&mut stdin.lock())
    }
}

/// Ask for one decision. A redirected session never waits for input.
pub fn approve(action: &PreparedAction, deadline: Instant) -> Result<Decision> {
    if !interactive() {
        return Err(noninteractive());
    }
    let style = Style::stderr();
    {
        let mut output = io::stderr().lock();
        preview(action, &mut output, &style)?;
    }
    read_before_deadline(deadline, move || {
        let mut answer = String::new();
        io::stdin()
            .lock()
            .take(MAX_ANSWER_BYTES)
            .read_line(&mut answer)?;
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
        return Err(timeout());
    }
    preview(action, output, &Style::plain())?;
    let mut answer = String::new();
    Read::take(input, MAX_ANSWER_BYTES)
        .read_line(&mut answer)
        .map_err(|error| KoruError::io("cannot read approval", error))?;
    if Instant::now() >= deadline {
        return Err(timeout());
    }
    Ok(parse_answer(&answer))
}

fn interactive() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

fn read_task_line(input: &mut impl BufRead) -> Result<String> {
    let mut task = String::new();
    Read::take(input, (MAX_TASK_BYTES + 2) as u64)
        .read_line(&mut task)
        .map_err(|error| KoruError::io("cannot read shell task", error))?;
    while task.ends_with(['\n', '\r']) {
        task.pop();
    }
    if task.len() > MAX_TASK_BYTES {
        return Err(KoruError::new(
            ErrorCode::BudgetExhausted,
            "shell task exceeds the 8192-byte limit",
        ));
    }
    if task.trim().is_empty() {
        return Err(KoruError::new(
            ErrorCode::Validation,
            "shell task must not be empty",
        ));
    }
    Ok(task)
}

fn preview(action: &PreparedAction, output: &mut impl Write, style: &Style) -> Result<()> {
    let heading = format!("Approve action {} for {}?", action.id(), action.command());
    writeln!(output, "{}", style.bold(&heading))
        .and_then(|()| writeln!(output, "{}", action.display_preview()))
        .and_then(|()| writeln!(output, "{}", style.cyan("Press y to run, n to deny:")))
        .and_then(|()| output.flush())
        .map_err(|error| KoruError::io("cannot display approval", error))?;
    Ok(())
}

fn parse_answer(answer: &str) -> Decision {
    let answer = answer.trim();
    if answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes") {
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

/// Enable UTF-8 erase handling for the duration of one read.
///
/// In canonical mode the line discipline removes one byte per erase unless
/// `IUTF8` is set, which leaves partial multi-byte characters behind. Setting
/// the flag makes backspace remove a whole character. The previous terminal
/// settings are always restored, and a non-terminal or unsupported platform is
/// left untouched.
#[cfg(unix)]
fn with_utf8_erase<T>(terminal: &impl std::os::fd::AsFd, body: impl FnOnce() -> T) -> T {
    use rustix::termios::{InputModes, OptionalActions, tcgetattr, tcsetattr};
    let previous = match tcgetattr(terminal) {
        Ok(settings) => settings,
        Err(_) => return body(),
    };
    if !previous.input_modes.contains(InputModes::IUTF8) {
        let mut updated = previous.clone();
        updated.input_modes.insert(InputModes::IUTF8);
        let _ = tcsetattr(terminal, OptionalActions::Now, &updated);
    }
    let result = body();
    let _ = tcsetattr(terminal, OptionalActions::Now, &previous);
    result
}

#[cfg(test)]
mod tests {
    use super::{parse_answer, read_before_deadline, read_task_line};
    use crate::{error::ErrorCode, permissions::Decision};
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

    #[test]
    fn task_input_is_bounded_and_rejects_redirected_input() {
        let mut input = std::io::Cursor::new(b"list files\n".to_vec());
        assert_eq!(read_task_line(&mut input).unwrap(), "list files");
        let mut input = std::io::Cursor::new(format!("{}\n", "x".repeat(8193)).into_bytes());
        assert_eq!(
            read_task_line(&mut input).unwrap_err().code(),
            ErrorCode::BudgetExhausted
        );
    }

    #[test]
    fn only_yes_answers_approve() {
        assert!(matches!(parse_answer("y\n"), Decision::ApproveOnce));
        assert!(matches!(parse_answer("YES\n"), Decision::ApproveOnce));
        assert!(matches!(parse_answer("n\n"), Decision::Deny));
        assert!(matches!(parse_answer(""), Decision::Deny));
    }
}
