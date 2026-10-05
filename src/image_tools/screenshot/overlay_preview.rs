use super::*;
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{
        FindWindowW, GetClientRect, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
        PostMessageW,
    },
};

fn handle(id: u64) -> Option<HWND> {
    let title = format!("Zi DevTools · 截图选择 · {id}\0")
        .encode_utf16()
        .collect::<Vec<_>>();
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if hwnd.is_null() {
        return None;
    }
    let mut owner = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut owner) };
    assert_eq!(
        owner,
        std::process::id(),
        "only this fixture's viewport may be targeted"
    );
    Some(hwnd)
}
impl State {
    pub fn preview_overlay_fixture(&mut self, ctx: &egui::Context, index: usize) {
        let display = crate::recorder::primary_display().unwrap();
        let image = RgbaImage::from_fn(display.width, display.height, |x, y| {
            image::Rgba([
                40 + (x / 40 % 100) as u8,
                60 + (y / 40 % 100) as u8,
                150,
                255,
            ])
        });
        self.next_overlay_id += 1;
        let mut overlay = overlay::Overlay::new(self.next_overlay_id, display, image, true, ctx);
        overlay.preview_fixture(index);
        self.overlay = Some(overlay);
    }
    pub fn preview_overlay_active(&self) -> bool {
        self.overlay.is_some() && handle(self.next_overlay_id).is_some()
    }
    pub fn preview_overlay_key(&self, key: u32, scan: u32) {
        let hwnd = handle(self.next_overlay_id).expect("owned overlay");
        unsafe {
            assert_ne!(
                PostMessageW(hwnd, 0x100, key as usize, (1 | (scan << 16)) as isize),
                0
            );
            assert_ne!(
                PostMessageW(
                    hwnd,
                    0x101,
                    key as usize,
                    (0xc0000001u32 | (scan << 16)) as isize
                ),
                0
            );
        }
    }
    pub fn preview_overlay_pointer(&self, x: i32, y: i32, kind: u8) {
        let hwnd = handle(self.next_overlay_id).expect("owned overlay");
        let point = ((y as u32 & 0xffff) << 16) | (x as u32 & 0xffff);
        let (message, keys) = match kind {
            0 => (0x201, 1),
            1 => (0x200, 1),
            2 => (0x202, 0),
            _ => panic!("invalid pointer event"),
        };
        unsafe {
            // Keep controlled WM positions independent of the user's real cursor.
            // Winit enables leave tracking on entry; cancel only this fixture's tracking/messages.
            use windows_sys::Win32::UI::{
                Input::KeyboardAndMouse::{
                    TME_CANCEL, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
                },
                WindowsAndMessaging::{PM_REMOVE, PeekMessageW, SendMessageW},
            };
            SendMessageW(hwnd, 0x200, keys, point as isize);
            let mut tracking = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_CANCEL | TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&mut tracking);
            let mut pending = std::mem::zeroed();
            while PeekMessageW(&mut pending, hwnd, 0x2a3, 0x2a3, PM_REMOVE) != 0 {}
            if message != 0x200 {
                assert_ne!(PostMessageW(hwnd, message, keys, point as isize), 0);
            }
        }
    }
    pub fn preview_overlay_dimensions(&self) -> [i32; 2] {
        let hwnd = handle(self.next_overlay_id).unwrap();
        let mut rect = unsafe { std::mem::zeroed() };
        assert_ne!(unsafe { GetClientRect(hwnd, &mut rect) }, 0);
        [rect.right - rect.left, rect.bottom - rect.top]
    }
    pub fn preview_overlay_check(&self, phase: u8) {
        match phase {
            0 => {
                let overlay = self.overlay.as_ref().unwrap();
                let hwnd = handle(overlay.id).unwrap();
                assert_ne!(unsafe { IsWindowVisible(hwnd) }, 0);
                let mut rect = unsafe { std::mem::zeroed() };
                assert_ne!(unsafe { GetWindowRect(hwnd, &mut rect) }, 0);
                assert_eq!(
                    (
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top
                    ),
                    (
                        overlay.display.x,
                        overlay.display.y,
                        overlay.display.width as i32,
                        overlay.display.height as i32
                    )
                );
                let mut cloak = 0u32;
                assert_eq!(
                    unsafe {
                        windows_sys::Win32::Graphics::Dwm::DwmGetWindowAttribute(
                            hwnd,
                            windows_sys::Win32::Graphics::Dwm::DWMWA_CLOAKED as u32,
                            &mut cloak as *mut _ as _,
                            4,
                        )
                    },
                    0
                );
                assert_eq!(cloak, 0);
            }
            1 => {
                assert!(self.overlay.is_none() && self.pending.is_none());
                assert_eq!(self.source.as_ref().unwrap().dimensions(), (800, 360));
                assert_eq!(self.output.as_ref().unwrap().dimensions(), (590, 200));
                assert!(self.dirty && self.has_work());
            }
            2 => {
                assert!(self.overlay.is_none() && self.pending.is_none());
                let image = self.output.as_ref().unwrap();
                assert!(
                    (i64::from(image.width()) - 1000).abs() <= 2
                        && (i64::from(image.height()) - 700).abs() <= 2,
                    "{:?}",
                    image.dimensions()
                );
                assert_eq!(image.get_pixel(100, 600)[3], 0);
                assert_eq!(image.get_pixel(900, 600)[3], 255);
            }
            3 => {
                assert!(
                    handle(self.next_overlay_id).is_none(),
                    "closed overlay HWND must be destroyed"
                );
            }
            4 => {
                let image = self.output.as_ref().unwrap();
                let source = self.source.as_ref().unwrap();
                assert_eq!(image.dimensions(), source.dimensions());
                assert_eq!(image.as_ref(), source.as_ref());
            }
            _ => panic!("invalid phase"),
        }
    }
}
