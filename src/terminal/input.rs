//! Interactive task input and one-use approval decisions.
//!
//! Both prompts require an interactive terminal on standard input and standard
//! error. The task prompt runs its own raw-mode line editor so deletion is
//! UTF-8- and display-width-aware: the kernel line discipline erases one byte
//! (or one code point) at a time and leaves half of a double-width character
//! behind, which makes CJK editing look broken. Approval reads one short answer
//! line and treats anything other than `y`/`yes` as a denial.
use super::style::Style;
use crate::{
    error::{ErrorCode, KoruError, Result},
    permissions::{Decision, PreparedAction},
};
#[cfg(unix)]
use std::os::fd::AsFd;
use std::{
    io::{self, BufRead, IsTerminal, Read, Write},
    sync::mpsc,
    thread,
    time::Instant,
};

const MAX_TASK_BYTES: usize = 8 * 1024;
const MAX_ANSWER_BYTES: u64 = 16;
const BACKSPACE: char = '\u{8}';
const DELETE: char = '\u{7f}';
const CANCEL: char = '\u{3}';
const END_OF_INPUT: char = '\u{4}';
const LINE_KILL: char = '\u{15}';
const ESCAPE: u8 = 0x1b;

/// How one raw-mode task read ended.
enum LineOutcome {
    /// The user submitted this text.
    Text(String),
    /// The user pressed Ctrl-C.
    Interrupted,
    /// The user pressed Ctrl-D on an empty line.
    EndOfInput,
    /// The line exceeded the byte limit.
    TooLong,
}

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
    let outcome = {
        let _guard = RawMode::enable(stdin.as_fd());
        edit_line(&mut stdin.lock(), &mut output)
    }
    .map_err(|error| KoruError::io("cannot read shell task", error))?;
    #[cfg(not(unix))]
    let outcome = edit_line(&mut stdin.lock(), &mut output)
        .map_err(|error| KoruError::io("cannot read shell task", error))?;
    match outcome {
        LineOutcome::Text(text) if !text.trim().is_empty() => Ok(text),
        LineOutcome::TooLong => Err(KoruError::new(
            ErrorCode::BudgetExhausted,
            "shell task exceeds the 8192-byte limit",
        )),
        LineOutcome::Interrupted => Err(KoruError::new(
            ErrorCode::Cancelled,
            "shell task input was cancelled",
        )),
        _ => Err(KoruError::new(
            ErrorCode::Validation,
            "shell task must not be empty",
        )),
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

/// Read and echo one line with UTF-8- and width-aware editing.
fn edit_line(reader: &mut impl Read, writer: &mut impl Write) -> io::Result<LineOutcome> {
    let mut buffer = String::new();
    let mut pending: Vec<u8> = Vec::new();
    loop {
        let Some(byte) = read_byte(reader)? else {
            if buffer.is_empty() {
                writer.flush()?;
                return Ok(LineOutcome::EndOfInput);
            }
            writer.write_all(b"\n")?;
            writer.flush()?;
            return Ok(LineOutcome::Text(buffer));
        };
        if byte == ESCAPE {
            // Drop arrow keys and other escape sequences instead of inserting
            // their bytes into the task.
            pending.clear();
            consume_escape(reader)?;
            continue;
        }
        pending.push(byte);
        let decoded = match std::str::from_utf8(&pending) {
            Ok(text) => text.to_owned(),
            Err(error) => {
                // `None` means the sequence is incomplete; keep reading.
                if error.error_len().is_some() {
                    pending.clear();
                }
                continue;
            }
        };
        pending.clear();
        for ch in decoded.chars() {
            match ch {
                '\r' | '\n' => {
                    writer.write_all(b"\n")?;
                    writer.flush()?;
                    return Ok(LineOutcome::Text(buffer));
                }
                BACKSPACE | DELETE => {
                    if let Some(removed) = buffer.pop() {
                        erase(writer, char_width(removed))?;
                    }
                }
                LINE_KILL => {
                    while let Some(removed) = buffer.pop() {
                        erase(writer, char_width(removed))?;
                    }
                }
                CANCEL => {
                    writer.write_all(b"^C\n")?;
                    writer.flush()?;
                    return Ok(LineOutcome::Interrupted);
                }
                END_OF_INPUT => {
                    if buffer.is_empty() {
                        writer.flush()?;
                        return Ok(LineOutcome::EndOfInput);
                    }
                }
                ch if ch.is_control() => {}
                ch => {
                    if buffer.len() + ch.len_utf8() > MAX_TASK_BYTES {
                        writer.flush()?;
                        return Ok(LineOutcome::TooLong);
                    }
                    let mut bytes = [0_u8; 4];
                    let encoded = ch.encode_utf8(&mut bytes);
                    buffer.push(ch);
                    writer.write_all(encoded.as_bytes())?;
                }
            }
        }
        writer.flush()?;
    }
}

fn read_byte(reader: &mut impl Read) -> io::Result<Option<u8>> {
    let mut byte = [0_u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => return Ok(None),
            Ok(_) => return Ok(Some(byte[0])),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

/// Consume one escape sequence so its bytes never reach the line buffer.
fn consume_escape(reader: &mut impl Read) -> io::Result<()> {
    let Some(next) = read_byte(reader)? else {
        return Ok(());
    };
    if next == b'[' || next == b'O' {
        // CSI/SS3 sequence: consume through its final byte (0x40..=0x7e).
        while let Some(byte) = read_byte(reader)? {
            if (0x40..=0x7e).contains(&byte) {
                break;
            }
        }
    }
    Ok(())
}

/// Erase one character of the given display width from the echoed line.
fn erase(writer: &mut impl Write, width: usize) -> io::Result<()> {
    for _ in 0..width {
        writer.write_all(b"\x08")?;
    }
    for _ in 0..width {
        writer.write_all(b" ")?;
    }
    for _ in 0..width {
        writer.write_all(b"\x08")?;
    }
    Ok(())
}

/// Terminal columns used by one character; enough for CJK and emoji.
fn char_width(ch: char) -> usize {
    let code = ch as u32;
    if code == 0 || is_zero_width(code) {
        0
    } else if is_wide(code) {
        2
    } else {
        1
    }
}

fn is_zero_width(code: u32) -> bool {
    (0x0300..=0x036F).contains(&code)
        || (0x0483..=0x0489).contains(&code)
        || (0x0591..=0x05BD).contains(&code)
        || (0x200B..=0x200F).contains(&code)
        || code == 0x200D
        || (0xFE00..=0xFE0F).contains(&code)
        || (0xFE20..=0xFE2F).contains(&code)
        || (0x1AB0..=0x1AFF).contains(&code)
        || (0x1DC0..=0x1DFF).contains(&code)
        || (0x20D0..=0x20FF).contains(&code)
}

fn is_wide(code: u32) -> bool {
    (0x1100..=0x115F).contains(&code)
        || (0x2E80..=0x303E).contains(&code)
        || (0x3041..=0x33FF).contains(&code)
        || (0x3400..=0x4DBF).contains(&code)
        || (0x4E00..=0x9FFF).contains(&code)
        || (0xA000..=0xA4CF).contains(&code)
        || (0xA960..=0xA97F).contains(&code)
        || (0xAC00..=0xD7A3).contains(&code)
        || (0xF900..=0xFAFF).contains(&code)
        || (0xFE10..=0xFE19).contains(&code)
        || (0xFE30..=0xFE6F).contains(&code)
        || (0xFF00..=0xFF60).contains(&code)
        || (0xFFE0..=0xFFE6).contains(&code)
        || (0x1F300..=0x1F64F).contains(&code)
        || (0x1F900..=0x1F9FF).contains(&code)
        || (0x20000..=0x2FFFD).contains(&code)
        || (0x30000..=0x3FFFD).contains(&code)
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

/// Raw terminal mode for the duration of one line edit.
///
/// Canonical mode cannot erase a double-width character correctly, so the task
/// editor takes over echoing. `ISIG` is cleared as well so Ctrl-C reaches the
/// editor and the previous settings are restored before the process continues.
#[cfg(unix)]
struct RawMode<'a> {
    fd: std::os::fd::BorrowedFd<'a>,
    previous: rustix::termios::Termios,
}

#[cfg(unix)]
impl<'a> RawMode<'a> {
    fn enable(fd: std::os::fd::BorrowedFd<'a>) -> Option<Self> {
        use rustix::termios::{
            InputModes, LocalModes, OptionalActions, SpecialCodeIndex, tcgetattr, tcsetattr,
        };
        let previous = tcgetattr(fd).ok()?;
        let mut raw = previous.clone();
        raw.local_modes.remove(
            LocalModes::ICANON
                | LocalModes::ECHO
                | LocalModes::ECHONL
                | LocalModes::ISIG
                | LocalModes::IEXTEN,
        );
        raw.input_modes.remove(
            InputModes::ICRNL
                | InputModes::INLCR
                | InputModes::IGNCR
                | InputModes::IXON
                | InputModes::IXOFF,
        );
        raw.special_codes[SpecialCodeIndex::VMIN] = 1;
        raw.special_codes[SpecialCodeIndex::VTIME] = 0;
        tcsetattr(fd, OptionalActions::Now, &raw).ok()?;
        Some(Self { fd, previous })
    }
}

#[cfg(unix)]
impl Drop for RawMode<'_> {
    fn drop(&mut self) {
        let _ = rustix::termios::tcsetattr(
            self.fd,
            rustix::termios::OptionalActions::Now,
            &self.previous,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{LineOutcome, edit_line, parse_answer, read_before_deadline};
    use crate::{error::ErrorCode, permissions::Decision};
    use std::{
        io::Cursor,
        sync::mpsc,
        time::{Duration, Instant},
    };

    fn edit(bytes: &[u8]) -> (LineOutcome, String) {
        let mut input = Cursor::new(bytes.to_vec());
        let mut output = Vec::new();
        let outcome = edit_line(&mut input, &mut output).unwrap();
        (outcome, String::from_utf8_lossy(&output).into_owned())
    }

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
    fn double_width_characters_can_be_deleted() {
        let (outcome, echoed) = edit("你好\u{7f}\n".as_bytes());
        assert!(
            matches!(outcome, LineOutcome::Text(text) if text == "你"),
            "expected only the first character to remain"
        );
        // Erasing a double-width character clears two columns twice over.
        assert!(echoed.contains("\x08\x08  \x08\x08"), "{echoed:?}");
        let (outcome, _) = edit("你好\u{7f}\u{7f}\n".as_bytes());
        assert!(matches!(outcome, LineOutcome::Text(t) if t.is_empty()));
    }

    #[test]
    fn ascii_editing_and_submission_work() {
        let (outcome, _) = edit(b"abc\x7f\n");
        assert!(matches!(outcome, LineOutcome::Text(text) if text == "ab"));
        let (outcome, _) = edit(b"abc\x15\n");
        assert!(matches!(outcome, LineOutcome::Text(text) if text.is_empty()));
    }

    #[test]
    fn control_keys_end_the_read_without_inserting_text() {
        let (outcome, _) = edit(b"abc\x03");
        assert!(matches!(outcome, LineOutcome::Interrupted));
        let (outcome, _) = edit(b"\x04");
        assert!(matches!(outcome, LineOutcome::EndOfInput));
    }

    #[test]
    fn escape_sequences_are_ignored() {
        let (outcome, _) = edit(b"\x1b[200~hi\x1b[201~\n");
        assert!(matches!(outcome, LineOutcome::Text(text) if text == "hi"));
        let (outcome, _) = edit(b"a\x1b[D\x1b[Cb\n");
        assert!(matches!(outcome, LineOutcome::Text(text) if text == "ab"));
    }

    #[test]
    fn oversized_input_is_rejected() {
        let mut bytes = vec![b'x'; 8193];
        bytes.push(b'\n');
        assert!(matches!(edit(&bytes).0, LineOutcome::TooLong));
    }

    #[test]
    fn only_yes_answers_approve() {
        assert!(matches!(parse_answer("y\n"), Decision::ApproveOnce));
        assert!(matches!(parse_answer("YES\n"), Decision::ApproveOnce));
        assert!(matches!(parse_answer("n\n"), Decision::Deny));
        assert!(matches!(parse_answer(""), Decision::Deny));
    }
}
