use super::*;

#[cfg(windows)]
fn handle(id: u64) -> windows_sys::Win32::Foundation::HWND {
    use windows_sys::Win32::{
        System::Threading::GetCurrentProcessId,
        UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId},
    };
    let title = format!("Zi DevTools · 时钟小窗 · {id}\0")
        .encode_utf16()
        .collect::<Vec<_>>();
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    assert!(!hwnd.is_null(), "native child viewport missing");
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    assert_eq!(
        pid,
        unsafe { GetCurrentProcessId() },
        "do not target another process"
    );
    hwnd
}
impl State {
    pub fn preview_window_prepare(&mut self, ctx: &egui::Context, index: usize) {
        let view = match index {
            0..=7 => index / 2,
            8..=9 | 12..=13 => 0,
            10..=11 => 2,
            _ => panic!("unknown window fixture"),
        };
        self.preview_fixture(if view == 3 { 4 } else { view });
        let target = match view {
            0 => Target::World(chrono_tz::Asia::Shanghai),
            1 => Target::Stopwatch,
            2 => Target::Timer(1),
            3 => Target::Focus,
            _ => unreachable!(),
        };
        self.open_window(target, ctx);
        if (8..=9).contains(&index) {
            self.windows.panes[0].fullscreen = true;
        }
        if (10..=11).contains(&index) {
            self.timers.remove(0);
        }
        if index >= 12 {
            self.windows.panes[0].size = egui::vec2(320.0, 260.0);
        }
    }
    #[cfg(windows)]
    pub fn preview_window_dimensions(&self) -> [i32; 2] {
        let hwnd = handle(self.windows.panes[0].id);
        let mut rect: windows_sys::Win32::Foundation::RECT = unsafe { std::mem::zeroed() };
        assert_ne!(
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect) },
            0
        );
        [rect.right - rect.left, rect.bottom - rect.top]
    }
    #[cfg(windows)]
    pub fn preview_window_click(&self, index: usize) {
        use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
        let hwnd = handle(self.windows.panes[0].id);
        let point = self.windows.rects[index].center() * self.windows.scale;
        assert!(point.is_finite() && point.x > 0.0 && point.y > 0.0);
        let location = ((point.y as isize) << 16) | (point.x as isize & 0xffff);
        unsafe {
            assert_ne!(PostMessageW(hwnd, 0x0200, 0, location), 0);
            assert_ne!(PostMessageW(hwnd, 0x0201, 1, location), 0);
            assert_ne!(PostMessageW(hwnd, 0x0202, 0, location), 0);
        }
    }
    #[cfg(windows)]
    pub fn preview_window_key(&self, key: u32, scan: u32) {
        let hwnd = handle(self.windows.panes[0].id);
        unsafe {
            assert_ne!(
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    hwnd,
                    0x0100,
                    key as usize,
                    (1 | (scan << 16)) as isize
                ),
                0
            );
            assert_ne!(
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    hwnd,
                    0x0101,
                    key as usize,
                    (0xc0000001u32 | (scan << 16)) as isize
                ),
                0
            );
        }
    }
    #[cfg(windows)]
    pub fn preview_window_check(&mut self, ctx: &egui::Context, phase: u8) -> isize {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetWindowLongPtrW, WS_EX_TOPMOST,
        };
        match phase {
            0 => {
                assert_eq!(self.windows.panes.len(), 1);
                assert!(!self.timers[0].running());
                return handle(self.windows.panes[0].id) as isize;
            }
            1 => assert!(self.timers[0].running()),
            2 => assert!(!self.timers[0].running()),
            3 => {
                assert!(self.windows.panes[0].pinned);
                let hwnd = handle(self.windows.panes[0].id);
                assert_ne!(
                    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd) },
                    0
                );
                let mut cloaked: u32 = 0;
                assert_eq!(
                    unsafe {
                        windows_sys::Win32::Graphics::Dwm::DwmGetWindowAttribute(
                            hwnd,
                            windows_sys::Win32::Graphics::Dwm::DWMWA_CLOAKED as u32,
                            &mut cloaked as *mut _ as _,
                            std::mem::size_of::<u32>() as u32,
                        )
                    },
                    0
                );
                assert_eq!(
                    cloaked, 0,
                    "child must remain uncloaked while workbench is hidden"
                );
                assert_ne!(
                    unsafe { GetWindowLongPtrW(handle(self.windows.panes[0].id), GWL_EXSTYLE) }
                        & WS_EX_TOPMOST as isize,
                    0
                );
            }
            4 => {
                use windows_sys::Win32::{
                    Foundation::RECT,
                    Graphics::Gdi::{
                        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
                    },
                    UI::WindowsAndMessaging::GetWindowRect,
                };
                assert!(self.windows.actual_fullscreen);
                let hwnd = handle(self.windows.panes[0].id);
                let mut rect: RECT = unsafe { std::mem::zeroed() };
                assert_ne!(unsafe { GetWindowRect(hwnd, &mut rect) }, 0);
                let mut monitor: MONITORINFO = unsafe { std::mem::zeroed() };
                monitor.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                assert_ne!(
                    unsafe {
                        GetMonitorInfoW(
                            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                            &mut monitor,
                        )
                    },
                    0
                );
                assert_eq!(
                    [rect.left, rect.top, rect.right, rect.bottom],
                    [
                        monitor.rcMonitor.left,
                        monitor.rcMonitor.top,
                        monitor.rcMonitor.right,
                        monitor.rcMonitor.bottom
                    ],
                    "fullscreen must occupy actual monitor bounds"
                );
                println!(
                    "PASS fullscreen measured against monitor: {}x{}",
                    rect.right - rect.left,
                    rect.bottom - rect.top
                );
            }
            5 => {
                assert!(!self.windows.actual_fullscreen);
                assert!(self.windows.panes[0].pinned);
                assert_ne!(
                    unsafe { GetWindowLongPtrW(handle(self.windows.panes[0].id), GWL_EXSTYLE) }
                        & WS_EX_TOPMOST as isize,
                    0
                );
            }
            6 => {
                assert!(self.timers[0].running());
                return handle(self.windows.panes[0].id) as isize;
            }
            7 => {
                assert!(self.windows.panes.is_empty());
                assert!(self.timers[0].running());
                self.open_window(Target::Timer(1), ctx);
                assert_eq!(self.windows.panes[0].id, 2);
            }
            8 => {
                assert!(self.timers[0].running());
                return handle(2) as isize;
            }
            9 => {
                assert!(self.windows.panes.is_empty());
                assert!(self.timers[0].running());
                println!(
                    "PASS native clock child: shared timer start/pause, actual topmost style, fullscreen roundtrip, main return, child close/reopen with unique viewport IDs and timer retained"
                );
            }
            _ => panic!("unknown native window phase"),
        }
        0
    }
}
