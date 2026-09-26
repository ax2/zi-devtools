//! Global shortcut registration lives on its own Windows message thread.
use serde::{Deserialize, Serialize};
use std::{sync::mpsc, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Setting {
    pub enabled: bool,
    pub shortcut: String,
}
impl Default for Setting {
    fn default() -> Self {
        Self {
            enabled: true,
            shortcut: "Ctrl+Alt+Space".into(),
        }
    }
}
pub fn parse(text: &str) -> Result<(u32, u32), String> {
    let mut modifiers = 0;
    let mut key = None;
    for part in text.split('+').map(|s| s.trim().to_ascii_uppercase()) {
        let flag = match part.as_str() {
            "CTRL" => 2,
            "ALT" => 1,
            "SHIFT" => 4,
            _ => 0,
        };
        if flag != 0 {
            if modifiers & flag != 0 {
                return Err("修饰键重复".into());
            }
            modifiers |= flag;
        } else {
            if key.is_some() {
                return Err("只能包含一个主键".into());
            }
            key = Some(match part.as_str() {
                "SPACE" => 32,
                s if s.len() == 1 && s.as_bytes()[0].is_ascii_uppercase() => s.as_bytes()[0] as u32,
                s if s.starts_with('F') => match s[1..].parse::<u32>() {
                    Ok(n @ 1..=12) => 111 + n,
                    _ => return Err("功能键仅支持 F1–F12".into()),
                },
                _ => return Err("主键支持 A–Z、F1–F12 或 Space".into()),
            });
        }
    }
    if modifiers & 3 == 0 {
        return Err("至少包含 Ctrl 或 Alt，避免占用普通输入".into());
    }
    Ok((modifiers, key.ok_or("缺少主键")?))
}
pub enum Event {
    Triggered,
    Configured(Setting, Result<(), String>),
}
enum Command {
    Configure(Setting),
    Stop,
}
pub struct Service {
    tx: mpsc::Sender<Command>,
    pub events: mpsc::Receiver<Event>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Service {
    pub fn new(setting: Setting, wake: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let (out, events) = mpsc::channel();
        let thread = std::thread::spawn(move || worker(rx, out, wake));
        let service = Self {
            tx,
            events,
            thread: Some(thread),
        };
        service.configure(setting);
        service
    }
    pub fn configure(&self, setting: Setting) {
        let _ = self.tx.send(Command::Configure(setting));
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[cfg(windows)]
fn worker(rx: mpsc::Receiver<Command>, out: mpsc::Sender<Event>, wake: impl Fn()) {
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey},
        WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_HOTKEY},
    };
    let mut active: Option<(i32, (u32, u32))> = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(25)) {
            Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Command::Configure(setting)) => {
                let result = if !setting.enabled {
                    if let Some((id, _)) = active.take() {
                        unsafe {
                            UnregisterHotKey(std::ptr::null_mut(), id);
                        }
                    }
                    Ok(())
                } else {
                    parse(&setting.shortcut).and_then(|keys| {
                        if active.is_some_and(|(_, old)| old == keys) {
                            return Ok(());
                        }
                        let next = if active.is_some_and(|(id, _)| id == 1) {
                            2
                        } else {
                            1
                        };
                        if unsafe {
                            RegisterHotKey(
                                std::ptr::null_mut(),
                                next,
                                keys.0 | MOD_NOREPEAT,
                                keys.1,
                            )
                        } == 0
                        {
                            return Err(
                                "快捷键被占用或注册失败；原有快捷键仍保留，请换一个组合".into()
                            );
                        }
                        if let Some((id, _)) = active {
                            unsafe {
                                UnregisterHotKey(std::ptr::null_mut(), id);
                            }
                        }
                        active = Some((next, keys));
                        Ok(())
                    })
                };
                if out.send(Event::Configured(setting, result)).is_err() {
                    break;
                }
                wake();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        while unsafe {
            PeekMessageW(
                &mut msg,
                std::ptr::null_mut(),
                WM_HOTKEY,
                WM_HOTKEY,
                PM_REMOVE,
            )
        } != 0
        {
            if active.is_some_and(|(id, _)| id as usize == msg.wParam) {
                let _ = out.send(Event::Triggered);
                wake();
            }
        }
    }
    if let Some((id, _)) = active {
        unsafe {
            UnregisterHotKey(std::ptr::null_mut(), id);
        }
    }
}
#[cfg(not(windows))]
fn worker(rx: mpsc::Receiver<Command>, out: mpsc::Sender<Event>, wake: impl Fn()) {
    while let Ok(Command::Configure(setting)) = rx.recv() {
        let _ = out.send(Event::Configured(
            setting,
            Err("系统热键暂仅支持 Windows".into()),
        ));
        wake();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcut_parser_rejects_ambiguous_and_plain_keys() {
        assert_eq!(parse(" ctrl + ALT + space ").unwrap(), (3, 32));
        assert_eq!(parse("Ctrl+Shift+F8").unwrap(), (6, 119));
        for bad in [
            "K",
            "Shift+K",
            "Ctrl+Ctrl+K",
            "Win+K",
            "Ctrl+A+B",
            "Ctrl+F13",
            "Alt+",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
    #[cfg(windows)]
    #[test]
    fn registration_conflict_keeps_old_binding_and_drop_releases_it() {
        let setting = Setting {
            enabled: true,
            shortcut: "Ctrl+Alt+Shift+F11".into(),
        };
        let one = Service::new(setting.clone(), || {});
        assert!(matches!(
            one.events.recv_timeout(Duration::from_secs(2)).unwrap(),
            Event::Configured(_, Ok(()))
        ));
        let two = Service::new(setting.clone(), || {});
        assert!(matches!(
            two.events.recv_timeout(Duration::from_secs(2)).unwrap(),
            Event::Configured(_, Err(_))
        ));
        one.configure(Setting {
            enabled: true,
            shortcut: "invalid".into(),
        });
        assert!(matches!(
            one.events.recv_timeout(Duration::from_secs(2)).unwrap(),
            Event::Configured(_, Err(_))
        ));
        two.configure(setting.clone());
        assert!(matches!(
            two.events.recv_timeout(Duration::from_secs(2)).unwrap(),
            Event::Configured(_, Err(_))
        ));
        drop(one);
        two.configure(setting);
        assert!(matches!(
            two.events.recv_timeout(Duration::from_secs(2)).unwrap(),
            Event::Configured(_, Ok(()))
        ));
    }
}
