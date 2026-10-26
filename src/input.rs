//! Translation of crossterm input events into the byte sequences a program in the PTY expects.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

/// xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl.
fn modifier_param(mods: KeyModifiers) -> u8 {
    let mut m = 1;
    if mods.contains(KeyModifiers::SHIFT) {
        m += 1;
    }
    if mods.contains(KeyModifiers::ALT) {
        m += 2;
    }
    if mods.contains(KeyModifiers::CONTROL) {
        m += 4;
    }
    m
}

fn ctrl_char(c: char) -> Option<u8> {
    match c {
        'a'..='z' | 'A'..='Z' => Some((c.to_ascii_lowercase() as u8) & 0x1f),
        '@' | ' ' | '2' => Some(0x00),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

/// Bytes to send to the PTY for a key press, or `None` if the key is not forwarded.
/// `app_cursor` is the DECCKM state of the running program.
pub fn key_to_bytes(key: &KeyEvent, app_cursor: bool) -> Option<Vec<u8>> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let mods = key.modifiers;
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT);
    let with_alt = |mut bytes: Vec<u8>| {
        if alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };
    let m = modifier_param(mods);

    let cursor = |letter: char| -> Vec<u8> {
        if m > 1 {
            format!("\x1b[1;{}{}", m, letter).into_bytes()
        } else if app_cursor {
            format!("\x1bO{}", letter).into_bytes()
        } else {
            format!("\x1b[{}", letter).into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if m > 1 {
            format!("\x1b[{};{}~", n, m).into_bytes()
        } else {
            format!("\x1b[{}~", n).into_bytes()
        }
    };

    let bytes = match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                match ctrl_char(c) {
                    Some(b) => with_alt(vec![b]),
                    None => with_alt(c.to_string().into_bytes()),
                }
            } else {
                with_alt(c.to_string().into_bytes())
            }
        }
        KeyCode::Enter => with_alt(b"\r".to_vec()),
        KeyCode::Backspace => with_alt(if ctrl { vec![0x08] } else { vec![0x7f] }),
        KeyCode::Tab => with_alt(b"\t".to_vec()),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => with_alt(vec![0x1b]),
        KeyCode::Null => vec![0],
        KeyCode::Up => cursor('A'),
        KeyCode::Down => cursor('B'),
        KeyCode::Right => cursor('C'),
        KeyCode::Left => cursor('D'),
        KeyCode::Home => cursor('H'),
        KeyCode::End => cursor('F'),
        KeyCode::Insert => tilde(2),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::F(n @ 1..=4) => {
            let letter = (b'P' + n - 1) as char;
            if m > 1 {
                format!("\x1b[1;{}{}", m, letter).into_bytes()
            } else {
                format!("\x1bO{}", letter).into_bytes()
            }
        }
        KeyCode::F(n) => {
            let code = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return None,
            };
            tilde(code)
        }
        _ => return None,
    };
    Some(bytes)
}

/// Wraps pasted text in bracketed-paste markers when the program asked for them.
pub fn paste_to_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    // Terminals send newlines as CR; strip any embedded end marker so the paste can't escape.
    let body = text
        .replace("\r\n", "\r")
        .replace('\n', "\r")
        .replace("\x1b[201~", "");
    if bracketed {
        format!("\x1b[200~{}\x1b[201~", body).into_bytes()
    } else {
        body.into_bytes()
    }
}

/// Encodes a mouse event for a program in the PTY. `col`/`row` are 1-based cell coordinates
/// relative to the terminal pane. Returns `None` if the program did not ask for this event or
/// the position cannot be encoded.
pub fn encode_mouse(
    kind: MouseEventKind,
    mods: KeyModifiers,
    col: u16,
    row: u16,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    use MouseProtocolMode as M;
    let button = |b: MouseButton| match b {
        MouseButton::Left => 0u16,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let sgr = encoding == MouseProtocolEncoding::Sgr;

    let (mut code, release) = match kind {
        MouseEventKind::Down(b) if mode != M::None => (button(b), false),
        MouseEventKind::Up(b)
            if matches!(mode, M::PressRelease | M::ButtonMotion | M::AnyMotion) =>
        {
            (if sgr { button(b) } else { 3 }, true)
        }
        MouseEventKind::Drag(b) if matches!(mode, M::ButtonMotion | M::AnyMotion) => {
            (button(b) + 32, false)
        }
        MouseEventKind::Moved if mode == M::AnyMotion => (3 + 32, false),
        MouseEventKind::ScrollUp if mode != M::None => (64, false),
        MouseEventKind::ScrollDown if mode != M::None => (65, false),
        MouseEventKind::ScrollLeft if mode != M::None => (66, false),
        MouseEventKind::ScrollRight if mode != M::None => (67, false),
        _ => return None,
    };

    // X10 mode reports no modifiers.
    if mode != M::Press {
        if mods.contains(KeyModifiers::SHIFT) {
            code += 4;
        }
        if mods.contains(KeyModifiers::ALT) {
            code += 8;
        }
        if mods.contains(KeyModifiers::CONTROL) {
            code += 16;
        }
    }

    match encoding {
        MouseProtocolEncoding::Sgr => Some(
            format!(
                "\x1b[<{};{};{}{}",
                code,
                col,
                row,
                if release { 'm' } else { 'M' }
            )
            .into_bytes(),
        ),
        MouseProtocolEncoding::Default => {
            if col > 223 || row > 223 {
                return None;
            }
            Some(vec![
                0x1b,
                b'[',
                b'M',
                (32 + code) as u8,
                (32 + col) as u8,
                (32 + row) as u8,
            ])
        }
        MouseProtocolEncoding::Utf8 => {
            let mut out = b"\x1b[M".to_vec();
            for v in [32 + code, 32 + col, 32 + row] {
                let c = char::from_u32(v as u32).filter(|_| v < 2048)?;
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            Some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> Vec<u8> {
        key_to_bytes(&KeyEvent::new(code, mods), false).unwrap()
    }

    #[test]
    fn ctrl_letters_send_control_bytes() {
        let c = KeyModifiers::CONTROL;
        assert_eq!(key(KeyCode::Char('d'), c), vec![0x04]);
        assert_eq!(key(KeyCode::Char('r'), c), vec![0x12]);
        assert_eq!(key(KeyCode::Char('l'), c), vec![0x0c]);
        assert_eq!(key(KeyCode::Char('a'), c), vec![0x01]);
        assert_eq!(key(KeyCode::Char('Z'), c), vec![0x1a]);
        assert_eq!(key(KeyCode::Char(' '), c), vec![0x00]);
        assert_eq!(key(KeyCode::Char('['), c), vec![0x1b]);
        assert_eq!(key(KeyCode::Char('_'), c), vec![0x1f]);
    }

    #[test]
    fn alt_prefixes_escape() {
        assert_eq!(key(KeyCode::Char('b'), KeyModifiers::ALT), b"\x1bb".to_vec());
        assert_eq!(
            key(KeyCode::Char('x'), KeyModifiers::ALT | KeyModifiers::CONTROL),
            vec![0x1b, 0x18]
        );
    }

    #[test]
    fn editing_keys() {
        let n = KeyModifiers::NONE;
        assert_eq!(key(KeyCode::Backspace, n), vec![0x7f]);
        assert_eq!(key(KeyCode::Backspace, KeyModifiers::CONTROL), vec![0x08]);
        assert_eq!(key(KeyCode::Enter, n), b"\r".to_vec());
        assert_eq!(key(KeyCode::BackTab, KeyModifiers::SHIFT), b"\x1b[Z".to_vec());
        assert_eq!(key(KeyCode::PageUp, n), b"\x1b[5~".to_vec());
        assert_eq!(key(KeyCode::PageDown, n), b"\x1b[6~".to_vec());
        assert_eq!(key(KeyCode::Insert, n), b"\x1b[2~".to_vec());
        assert_eq!(key(KeyCode::Delete, KeyModifiers::CONTROL), b"\x1b[3;5~".to_vec());
        assert_eq!(key(KeyCode::Char('é'), n), "é".as_bytes().to_vec());
    }

    #[test]
    fn cursor_keys_follow_application_mode_and_modifiers() {
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(key_to_bytes(&up, false).unwrap(), b"\x1b[A".to_vec());
        assert_eq!(key_to_bytes(&up, true).unwrap(), b"\x1bOA".to_vec());
        assert_eq!(key(KeyCode::Left, KeyModifiers::CONTROL), b"\x1b[1;5D".to_vec());
        assert_eq!(key(KeyCode::End, KeyModifiers::SHIFT), b"\x1b[1;2F".to_vec());
    }

    #[test]
    fn function_keys() {
        let n = KeyModifiers::NONE;
        assert_eq!(key(KeyCode::F(1), n), b"\x1bOP".to_vec());
        assert_eq!(key(KeyCode::F(4), n), b"\x1bOS".to_vec());
        assert_eq!(key(KeyCode::F(5), n), b"\x1b[15~".to_vec());
        assert_eq!(key(KeyCode::F(12), n), b"\x1b[24~".to_vec());
        assert_eq!(key(KeyCode::F(2), KeyModifiers::SHIFT), b"\x1b[1;2Q".to_vec());
        assert!(key_to_bytes(&KeyEvent::new(KeyCode::F(13), n), false).is_none());
    }

    #[test]
    fn release_events_are_ignored() {
        let mut ev = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        ev.kind = KeyEventKind::Release;
        assert!(key_to_bytes(&ev, false).is_none());
    }

    #[test]
    fn paste_is_bracketed_only_when_requested() {
        assert_eq!(paste_to_bytes("a\nb", false), b"a\rb".to_vec());
        assert_eq!(paste_to_bytes("a", true), b"\x1b[200~a\x1b[201~".to_vec());
        assert_eq!(
            paste_to_bytes("x\x1b[201~rm", true),
            b"\x1b[200~xrm\x1b[201~".to_vec()
        );
    }

    use MouseProtocolEncoding as E;
    use MouseProtocolMode as M;

    fn mouse(kind: MouseEventKind, mode: M, enc: E) -> Option<Vec<u8>> {
        encode_mouse(kind, KeyModifiers::NONE, 10, 5, mode, enc)
    }

    #[test]
    fn sgr_buttons_are_not_swapped() {
        let d = |b| mouse(MouseEventKind::Down(b), M::PressRelease, E::Sgr).unwrap();
        assert_eq!(d(MouseButton::Left), b"\x1b[<0;10;5M".to_vec());
        assert_eq!(d(MouseButton::Middle), b"\x1b[<1;10;5M".to_vec());
        assert_eq!(d(MouseButton::Right), b"\x1b[<2;10;5M".to_vec());
        assert_eq!(
            mouse(MouseEventKind::Up(MouseButton::Right), M::PressRelease, E::Sgr).unwrap(),
            b"\x1b[<2;10;5m".to_vec()
        );
    }

    #[test]
    fn events_respect_requested_mode() {
        let moved = MouseEventKind::Moved;
        let drag = MouseEventKind::Drag(MouseButton::Left);
        let up = MouseEventKind::Up(MouseButton::Left);
        assert!(mouse(moved, M::PressRelease, E::Sgr).is_none());
        assert!(mouse(moved, M::ButtonMotion, E::Sgr).is_none());
        assert_eq!(mouse(moved, M::AnyMotion, E::Sgr).unwrap(), b"\x1b[<35;10;5M".to_vec());
        assert!(mouse(drag, M::PressRelease, E::Sgr).is_none());
        assert_eq!(mouse(drag, M::ButtonMotion, E::Sgr).unwrap(), b"\x1b[<32;10;5M".to_vec());
        assert!(mouse(up, M::Press, E::Sgr).is_none());
        assert!(mouse(MouseEventKind::Down(MouseButton::Left), M::None, E::Sgr).is_none());
    }

    #[test]
    fn default_and_utf8_encodings() {
        let down = MouseEventKind::Down(MouseButton::Left);
        assert_eq!(
            mouse(down, M::PressRelease, E::Default).unwrap(),
            vec![0x1b, b'[', b'M', 32, 42, 37]
        );
        assert_eq!(
            mouse(MouseEventKind::Up(MouseButton::Left), M::PressRelease, E::Default).unwrap(),
            vec![0x1b, b'[', b'M', 35, 42, 37]
        );
        assert_eq!(
            mouse(MouseEventKind::ScrollUp, M::Press, E::Default).unwrap(),
            vec![0x1b, b'[', b'M', 96, 42, 37]
        );
        assert!(
            encode_mouse(down, KeyModifiers::NONE, 300, 5, M::PressRelease, E::Default).is_none()
        );
        let utf8 = encode_mouse(down, KeyModifiers::NONE, 300, 5, M::PressRelease, E::Utf8).unwrap();
        assert_eq!(&utf8[..4], b"\x1b[M ");
        assert_eq!(std::str::from_utf8(&utf8[4..]).unwrap(), "\u{14c}%");
    }

    #[test]
    fn modifiers_are_encoded_except_in_x10_mode() {
        let down = MouseEventKind::Down(MouseButton::Left);
        let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        assert_eq!(
            encode_mouse(down, ctrl_shift, 1, 1, M::PressRelease, E::Sgr).unwrap(),
            b"\x1b[<20;1;1M".to_vec()
        );
        assert_eq!(
            encode_mouse(down, ctrl_shift, 1, 1, M::Press, E::Sgr).unwrap(),
            b"\x1b[<0;1;1M".to_vec()
        );
    }
}
