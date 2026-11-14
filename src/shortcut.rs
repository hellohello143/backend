use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

fn down(key: i32) -> bool {
    unsafe { GetAsyncKeyState(key) < 0 }
}

fn key_name(key: i32) -> Option<String> {
    Some(match key {
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(key as u32)?.to_string(),
        0x60..=0x69 => format!("Numpad{}", key - 0x60),
        0x70..=0x87 => format!("F{}", key - 0x6F),
        _ => match key {
            0x08 => "Backspace",
            0x09 => "Tab",
            0x0D => "Enter",
            0x13 => "Pause",
            0x14 => "CapsLock",
            0x20 => "Space",
            0x21 => "PageUp",
            0x22 => "PageDown",
            0x23 => "End",
            0x24 => "Home",
            0x25 => "Left",
            0x26 => "Up",
            0x27 => "Right",
            0x28 => "Down",
            0x2C => "PrintScreen",
            0x2D => "Insert",
            0x2E => "Delete",
            0x6A => "NumpadMultiply",
            0x6B => "NumpadAdd",
            0x6D => "NumpadSubtract",
            0x6E => "NumpadDecimal",
            0x6F => "NumpadDivide",
            0x90 => "NumLock",
            0x91 => "ScrollLock",
            0xBA => "Semicolon",
            0xBB => "=",
            0xBC => "Comma",
            0xBD => "Minus",
            0xBE => "Period",
            0xBF => "Slash",
            0xC0 => "Backquote",
            0xDB => "BracketLeft",
            0xDC => "Backslash",
            0xDD => "BracketRight",
            0xDE => "Quote",
            _ => return None,
        }
        .to_owned(),
    })
}

pub enum Input {
    Cancel,
    Chord(String),
}

pub struct Recorder {
    pub index: usize,
    held: Vec<i32>,
}

impl Recorder {
    pub fn new(index: usize) -> Self {
        Self {
            index,
            held: (8..=254).filter(|key| down(*key)).collect(),
        }
    }

    pub fn modifiers(&self) -> Vec<&'static str> {
        [
            (down(0x11), "Ctrl"),
            (down(0x12), "Alt"),
            (down(0x10), "Shift"),
            (down(0x5B) || down(0x5C), "Super"),
        ]
        .into_iter()
        .filter_map(|(pressed, label)| pressed.then_some(label))
        .collect()
    }

    pub fn poll(&mut self, ctx: &eframe::egui::Context) -> Option<Input> {
        use eframe::egui::{Event, Key};
        let events = ctx.input(|input| input.events.clone());
        let now: Vec<_> = (8..=254).filter(|key| down(*key)).collect();
        let pressed = |key| now.contains(&key) && !self.held.contains(&key);
        let physical = now
            .iter()
            .copied()
            .find(|key| pressed(*key) && key_name(*key).is_some());
        let escape = pressed(0x1B)
            || events.iter().any(|event| {
                matches!(
                    event,
                    Event::Key {
                        key: Key::Escape,
                        pressed: true,
                        repeat: false,
                        ..
                    }
                )
            });
        let modifiers = self.modifiers();
        self.held = now;
        if escape {
            return Some(Input::Cancel);
        }
        let mut keys: Vec<_> = modifiers.into_iter().map(str::to_owned).collect();
        if let Some(key) = physical.and_then(key_name) {
            keys.push(key);
            return Some(Input::Chord(keys.join("+")));
        }
        // A quick tap can already be released when the UI polls Windows.
        for event in events {
            if let Event::Key {
                key,
                physical_key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } = event
            {
                let key = match physical_key.unwrap_or(key) {
                    Key::OpenBracket => "BracketLeft",
                    Key::CloseBracket => "BracketRight",
                    Key::Backtick => "Backquote",
                    Key::Equals => "=",
                    key => key.name(),
                };
                let prefix = [
                    (modifiers.ctrl, "Ctrl"),
                    (modifiers.alt, "Alt"),
                    (modifiers.shift, "Shift"),
                ]
                .into_iter()
                .filter_map(|(pressed, name)| pressed.then_some(name));
                let chord = prefix
                    .chain(std::iter::once(key))
                    .collect::<Vec<_>>()
                    .join("+");
                if chord.parse::<global_hotkey::hotkey::HotKey>().is_ok() {
                    return Some(Input::Chord(chord));
                }
            }
        }
        None
    }
}
