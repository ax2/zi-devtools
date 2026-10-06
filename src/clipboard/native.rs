//! A message-only window owns the opt-in Windows listener; no active polling.
use super::{BYTE_LIMIT, CapturePolicy, Picture, TEXT_LIMIT};
#[cfg(feature = "ui-preview")]
static TEXT_READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "ui-preview")]
static IMAGE_READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
use crossbeam_channel::{Receiver, bounded};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{DataExchange::*, Memory::*, Threading::*},
    UI::WindowsAndMessaging::*,
};

pub enum Event {
    Text(String, String),
    Error(String),
    Excluded,
    Image(Arc<Picture>, String),
}
enum Raw {
    Event(Event),
    Image(Vec<u8>, bool, String),
}
struct Reservation {
    bytes: usize,
    total: Arc<std::sync::atomic::AtomicUsize>,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.total.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
pub struct Queued {
    pub event: Event,
    _reservation: Reservation,
}
fn reserve(total: &Arc<std::sync::atomic::AtomicUsize>, bytes: usize) -> Option<Reservation> {
    let result = total.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
        value.checked_add(bytes).filter(|n| *n <= BYTE_LIMIT)
    });
    result.ok().map(|_| Reservation {
        bytes,
        total: total.clone(),
    })
}
pub struct Listener {
    pub rx: Receiver<Queued>,
    thread: u32,
    join: Option<std::thread::JoinHandle<()>>,
    gate: Arc<Mutex<()>>,
    policy: Arc<Mutex<CapturePolicy>>,
    own_sequence: Arc<AtomicU32>,
    pub paused: Arc<std::sync::atomic::AtomicBool>,
    pub lost: Arc<AtomicU32>,
    window: usize,
    images: Arc<std::sync::atomic::AtomicBool>,
    generation: Arc<std::sync::atomic::AtomicU64>,
}
impl Listener {
    pub fn start(ctx: eframe::egui::Context, policy: CapturePolicy) -> Result<Self, String> {
        Self::start_impl(ctx, None, policy)
    }
    fn start_impl(
        ctx: eframe::egui::Context,
        desktop: Option<usize>,
        policy: CapturePolicy,
    ) -> Result<Self, String> {
        let images = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_images = images.clone();
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let thread_generation = generation.clone();
        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let policy = Arc::new(Mutex::new(policy.normalized()?));
        let thread_policy = policy.clone();
        let (tx, rx) = bounded(64);
        let (ready_tx, ready_rx) = bounded(1);
        let gate = Arc::new(Mutex::new(()));
        let own_sequence = Arc::new(AtomicU32::new(0));
        let thread_gate = gate.clone();
        let own = own_sequence.clone();
        let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_paused = paused.clone();
        let lost = Arc::new(AtomicU32::new(0));
        let thread_lost = lost.clone();
        let join = std::thread::spawn(move || unsafe {
            #[cfg(feature = "ui-preview")]
            if let Some(desktop) = desktop {
                if windows_sys::Win32::System::StationsAndDesktops::SetThreadDesktop(desktop as _)
                    == 0
                {
                    let _ = ready_tx.send(Err("隔离测试线程桌面设置失败".into()));
                    return;
                }
            }
            #[cfg(not(feature = "ui-preview"))]
            let _ = desktop;
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let window = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            if window.is_null() || AddClipboardFormatListener(window) == 0 {
                if !window.is_null() {
                    DestroyWindow(window);
                }
                let _ = ready_tx.send(Err("无法注册Windows剪贴板通知".to_string()));
                return;
            }
            own.store(GetClipboardSequenceNumber(), Ordering::Release);
            let _ = ready_tx.send(Ok((GetCurrentThreadId(), window as usize)));
            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                if message.message == WM_CLIPBOARDUPDATE && !thread_paused.load(Ordering::Acquire) {
                    let (raw, epoch) = {
                        let _guard = thread_gate.lock().unwrap_or_else(|e| e.into_inner());
                        let sequence = GetClipboardSequenceNumber();
                        let epoch = thread_generation.load(Ordering::Acquire);
                        let raw = if thread_paused.load(Ordering::Acquire)
                            || sequence == own.load(Ordering::Acquire)
                        {
                            None
                        } else {
                            match read_text(
                                sequence,
                                &thread_policy.lock().unwrap_or_else(|e| e.into_inner()),
                                thread_images.load(Ordering::Acquire),
                            ) {
                                Ok(event) => event,
                                Err(error) => Some(Raw::Event(Event::Error(error))),
                            }
                        };
                        (raw, epoch)
                    };
                    // Clipboard lock and UI gate are released before decoding any pixels.
                    let event = match raw {
                        Some(Raw::Event(event)) => Some(event),
                        Some(Raw::Image(bytes, png, source)) => Some(
                            match if png {
                                Picture::from_png(&bytes)
                            } else {
                                super::media::from_dib(&bytes)
                            } {
                                Ok(value) => Event::Image(Arc::new(value), source),
                                Err(_) => Event::Error(
                                    "图片格式、尺寸、容量或位图布局不支持；未采集".into(),
                                ),
                            },
                        ),
                        None => None,
                    };
                    if let Some(event) = event {
                        let _guard = thread_gate.lock().unwrap_or_else(|e| e.into_inner());
                        if thread_generation.load(Ordering::Acquire) != epoch
                            || thread_paused.load(Ordering::Acquire)
                        {
                            continue;
                        }
                        let bytes = match &event {
                            Event::Text(text, _) => text.len(),
                            Event::Image(image, _) => image.cost(),
                            _ => 0,
                        };
                        if let Some(reservation) = reserve(&budget, bytes) {
                            if tx
                                .try_send(Queued {
                                    event,
                                    _reservation: reservation,
                                })
                                .is_err()
                            {
                                thread_lost.fetch_add(1, Ordering::Relaxed);
                            }
                        } else {
                            thread_lost.fetch_add(1, Ordering::Relaxed);
                        }
                        ctx.request_repaint();
                    }
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            RemoveClipboardFormatListener(window);
            DestroyWindow(window);
        });
        let (thread, window) = match ready_rx.recv() {
            Ok(Ok(id)) => id,
            Ok(Err(e)) => {
                let _ = join.join();
                return Err(e);
            }
            Err(_) => {
                let _ = join.join();
                return Err("Windows监听初始化失败".into());
            }
        };
        Ok(Self {
            rx,
            thread,
            join: Some(join),
            gate,
            policy,
            own_sequence,
            paused,
            lost,
            window,
            images,
            generation,
        })
    }
    pub fn set_policy(&self, policy: CapturePolicy) {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        self.generation.fetch_add(1, Ordering::AcqRel);
        *self.policy.lock().unwrap_or_else(|e| e.into_inner()) = policy;
        self.own_sequence
            .store(unsafe { GetClipboardSequenceNumber() }, Ordering::Release);
        while self.rx.try_recv().is_ok() {}
    }
    pub fn set_paused(&self, value: bool) {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.paused.store(value, Ordering::Release);
        self.own_sequence
            .store(unsafe { GetClipboardSequenceNumber() }, Ordering::Release);
        while self.rx.try_recv().is_ok() {}
    }
    pub fn set_images(&self, value: bool) {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.images.store(value, Ordering::Release);
        self.own_sequence
            .store(unsafe { GetClipboardSequenceNumber() }, Ordering::Release);
        while self.rx.try_recv().is_ok() {}
    }
    pub fn copy_picture(&self, picture: &Picture, dib: &[u8]) -> Result<(), String> {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let (seq, compatible) = write_picture(picture, dib, self.window as HWND)?;
        self.own_sequence.store(seq, Ordering::Release);
        if compatible {
            Ok(())
        } else {
            Err("PNG已复制，但兼容位图写入失败".into())
        }
    }
    pub fn copy(&self, text: &str) -> Result<(), String> {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let seq = write_text(text, self.window as HWND)?;
        self.own_sequence.store(seq, Ordering::Release);
        Ok(())
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        unsafe {
            PostThreadMessageW(self.thread, WM_QUIT, 0, 0);
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
struct Clipboard;
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}
unsafe fn open_clipboard() -> Result<Clipboard, String> {
    for _ in 0..5 {
        if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
            return Ok(Clipboard);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Err("剪贴板被其他程序占用，本次未读取/写入".into())
}
unsafe fn read_text(
    sequence: u32,
    policy: &CapturePolicy,
    images: bool,
) -> Result<Option<Raw>, String> {
    let png = png_format()?;
    let image_format = if images {
        [png, 17, 8]
            .into_iter()
            .find(|format| unsafe { IsClipboardFormatAvailable(*format) } != 0)
    } else {
        None
    };
    if image_format.is_none() && unsafe { IsClipboardFormatAvailable(13) } == 0 {
        return Ok(None);
    }
    let _opened = unsafe { open_clipboard() }?;
    if unsafe { GetClipboardSequenceNumber() } != sequence {
        return Ok(None);
    }
    let source = unsafe { source_name() };
    if policy.excludes(&source) {
        return Ok(Some(Raw::Event(Event::Excluded)));
    }
    if let Some(format) = image_format {
        #[cfg(feature = "ui-preview")]
        IMAGE_READS.fetch_add(1, Ordering::Relaxed);
        let handle = unsafe { GetClipboardData(format) };
        if handle.is_null() {
            return Err("图片格式读取失败".into());
        }
        let size = unsafe { GlobalSize(handle) };
        if size == 0 || size > super::media::RAW_LIMIT {
            return Err("剪贴板图片数据超过32MiB或为空".into());
        }
        let ptr = unsafe { GlobalLock(handle) } as *const u8;
        if ptr.is_null() {
            return Err("无法锁定图片数据".into());
        }
        let bytes = unsafe { std::slice::from_raw_parts(ptr, size) }.to_vec();
        unsafe {
            GlobalUnlock(handle);
        }
        return Ok(Some(Raw::Image(bytes, format == png, source)));
    }
    #[cfg(feature = "ui-preview")]
    TEXT_READS.fetch_add(1, Ordering::Relaxed);
    let handle = unsafe { GetClipboardData(13) };
    if handle.is_null() {
        return Err("文本格式读取失败".into());
    }
    let size = unsafe { GlobalSize(handle) };
    if !(2..=TEXT_LIMIT * 2 + 2).contains(&size) || size % 2 != 0 {
        return Err("文本分配大小无效或超出上限，未采集".into());
    }
    let ptr = unsafe { GlobalLock(handle) } as *const u16;
    if ptr.is_null() {
        return Err("无法锁定剪贴板文本".into());
    }
    let words = unsafe { std::slice::from_raw_parts(ptr, size / 2) };
    let decoded = words
        .iter()
        .position(|w| *w == 0)
        .ok_or("文本未结束，未采集")
        .and_then(|end| String::from_utf16(&words[..end]).map_err(|_| "文本UTF-16无效，未采集"));
    unsafe {
        GlobalUnlock(handle);
    }
    let text = decoded.map_err(str::to_string)?;
    if text.len() > TEXT_LIMIT {
        return Err("单条文本超过1 MiB，未采集".into());
    }
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(Raw::Event(Event::Text(text, source))))
}
unsafe fn source_name() -> String {
    let owner = unsafe { GetClipboardOwner() };
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(owner, &mut pid);
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return "来源未知".into();
    }
    let mut buffer = vec![0u16; 32768];
    let mut len = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut len) };
    unsafe {
        CloseHandle(handle);
    }
    if ok == 0 {
        return "来源未知".into();
    }
    let path = String::from_utf16_lossy(&buffer[..len as usize]);
    path.rsplit(['\\', '/'])
        .next()
        .unwrap_or("来源未知")
        .to_string()
}
fn write_text(text: &str, window: HWND) -> Result<u32, String> {
    if text.len() > TEXT_LIMIT || text.contains('\0') {
        return Err("复制文本超限或包含零字符".into());
    }
    let words: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, words.len() * 2);
        if memory.is_null() {
            return Err("剪贴板内存分配失败".into());
        }
        let ptr = GlobalLock(memory) as *mut u16;
        if ptr.is_null() {
            GlobalFree(memory);
            return Err("剪贴板内存锁定失败".into());
        }
        std::ptr::copy_nonoverlapping(words.as_ptr(), ptr, words.len());
        GlobalUnlock(memory);
        let opened = if OpenClipboard(window) != 0 {
            Ok(Clipboard)
        } else {
            Err("剪贴板被其他程序占用，未复制".into())
        };
        if let Err(e) = opened {
            GlobalFree(memory);
            return Err(e);
        }
        let _opened = opened.unwrap();
        if EmptyClipboard() == 0 || SetClipboardData(13, memory).is_null() {
            GlobalFree(memory);
            return Err("剪贴板写入失败".into());
        }
        drop(_opened);
        Ok(GetClipboardSequenceNumber())
    }
}

fn png_format() -> Result<u32, String> {
    let name: Vec<u16> = "PNG\0".encode_utf16().collect();
    let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
    if format == 0 {
        Err("无法注册PNG剪贴板格式".into())
    } else {
        Ok(format)
    }
}
fn memory(bytes: &[u8]) -> Result<*mut std::ffi::c_void, String> {
    unsafe {
        let value = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
        if value.is_null() {
            return Err("图片内存分配失败".into());
        }
        let ptr = GlobalLock(value);
        if ptr.is_null() {
            GlobalFree(value);
            return Err("图片内存锁定失败".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
        GlobalUnlock(value);
        Ok(value)
    }
}
fn write_picture(picture: &Picture, dib: &[u8], window: HWND) -> Result<(u32, bool), String> {
    let format = png_format()?;
    let png = memory(&picture.png)?;
    let bitmap = match memory(dib) {
        Ok(value) => value,
        Err(error) => {
            unsafe {
                GlobalFree(png);
            }
            return Err(error);
        }
    };
    unsafe {
        if OpenClipboard(window) == 0 {
            GlobalFree(png);
            GlobalFree(bitmap);
            return Err("剪贴板正被占用，未复制图片".into());
        }
        let opened = Clipboard;
        if EmptyClipboard() == 0 {
            GlobalFree(png);
            GlobalFree(bitmap);
            return Err("无法清空剪贴板，未复制图片".into());
        }
        if SetClipboardData(format, png).is_null() {
            GlobalFree(png);
            GlobalFree(bitmap);
            return Err("PNG写入失败".into());
        }
        let extra = SetClipboardData(17, bitmap);
        if extra.is_null() {
            GlobalFree(bitmap);
        }
        drop(opened);
        Ok((GetClipboardSequenceNumber(), !extra.is_null()))
    }
}
pub(super) fn copy_picture(picture: &Picture, dib: &[u8]) -> Result<(), String> {
    unsafe {
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if window.is_null() {
            return Err("图片复制窗口创建失败".into());
        }
        let result = write_picture(picture, dib, window).and_then(|(_, compatible)| {
            if compatible {
                Ok(())
            } else {
                Err("PNG已复制，但兼容位图写入失败".into())
            }
        });
        DestroyWindow(window);
        result
    }
}

/// Test-only private window station has its own clipboard; never use the interactive one.
#[cfg(feature = "ui-preview")]
pub fn isolated_fixture() -> Result<(), String> {
    use std::time::Duration;
    use windows_sys::Win32::System::StationsAndDesktops::*;
    struct Station {
        old: HWINSTA,
        old_desktop: HDESK,
        station: HWINSTA,
        desktop: HDESK,
    }
    impl Drop for Station {
        fn drop(&mut self) {
            unsafe {
                SetProcessWindowStation(self.old);
                SetThreadDesktop(self.old_desktop);
                if !self.desktop.is_null() {
                    CloseDesktop(self.desktop);
                }
                CloseWindowStation(self.station);
            }
        }
    }
    let name = format!("ZiDevToolsClipboardFixture-{}", uuid::Uuid::new_v4());
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut station = unsafe {
        let old = GetProcessWindowStation();
        let old_desktop = GetThreadDesktop(GetCurrentThreadId());
        let handle = CreateWindowStationW(wide.as_ptr(), 0, 0x10000000, std::ptr::null());
        if handle.is_null() {
            return Err(format!(
                "Private window station creation failed ({}); no clipboard access",
                GetLastError()
            ));
        }
        Station {
            old,
            old_desktop,
            station: handle,
            desktop: std::ptr::null_mut(),
        }
    };
    if unsafe { SetProcessWindowStation(station.station) } == 0 {
        return Err("Private window station selection failed; no clipboard access".into());
    }
    let desktop_name: Vec<u16> = "Default\0".encode_utf16().collect();
    station.desktop = unsafe {
        CreateDesktopW(
            desktop_name.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            0x10000000,
            std::ptr::null(),
        )
    };
    if station.desktop.is_null() || unsafe { SetThreadDesktop(station.desktop) } == 0 {
        return Err("Private desktop selection failed; no clipboard access".into());
    }
    if unsafe { GetProcessWindowStation() } != station.station
        || unsafe { GetThreadDesktop(GetCurrentThreadId()) } != station.desktop
    {
        return Err("Private station verification failed; no clipboard access".into());
    }
    let listener = Listener::start_impl(
        eframe::egui::Context::default(),
        Some(station.desktop as usize),
        CapturePolicy::default(),
    )?;
    let receive = |expected: &str| -> Result<(), String> {
        match listener
            .rx
            .recv_timeout(Duration::from_secs(2))
            .map(|queued| queued.event)
        {
            Ok(Event::Text(text, _)) if text == expected => Ok(()),
            _ => Err("Isolated clipboard notification/text mismatch".into()),
        }
    };
    write_text("合成通知：甲\n乙", listener.window as HWND)?;
    receive("合成通知：甲\n乙")?;
    listener.set_paused(true);
    write_text("暂停期间合成内容", listener.window as HWND)?;
    std::thread::sleep(Duration::from_millis(80));
    assert!(listener.rx.try_recv().is_err());
    listener.set_paused(false);
    std::thread::sleep(Duration::from_millis(50));
    assert!(listener.rx.try_recv().is_err());
    listener.copy("本工具合成输出")?;
    std::thread::sleep(Duration::from_millis(80));
    assert!(listener.rx.try_recv().is_err());
    write_text("本工具合成输出", listener.window as HWND)?;
    receive("本工具合成输出")?;
    let owner = unsafe { source_name() };
    assert_ne!(owner, "来源未知", "fixture process owner must be known");
    let policy = CapturePolicy::parse(&owner, false)?;
    listener.set_policy(policy);
    let before = TEXT_READS.load(Ordering::Acquire);
    write_text("被排除合成文本", listener.window as HWND)?;
    assert!(matches!(
        listener
            .rx
            .recv_timeout(Duration::from_secs(2))
            .map(|queued| queued.event),
        Ok(Event::Excluded)
    ));
    assert_eq!(
        TEXT_READS.load(Ordering::Acquire),
        before,
        "excluded source never retrieves text handle"
    );
    listener.set_policy(CapturePolicy::default());
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        listener.rx.try_recv().is_err(),
        "policy change does not catch up"
    );
    write_text("被排除合成文本", listener.window as HWND)?;
    receive("被排除合成文本")?;
    assert!(TEXT_READS.load(Ordering::Acquire) > before);
    let picture = Picture::from_image(image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(
        3,
        2,
        |x, y| image::Rgba([x as u8, y as u8, 140, (x * 80) as u8]),
    )))
    .map_err(|_| "fixture PNG encode")?;
    let dib = super::media::to_dib(&picture).map_err(|_| "fixture DIB encode")?;
    write_picture(&picture, &dib, listener.window as HWND)?;
    std::thread::sleep(Duration::from_millis(100));
    assert!(listener.rx.try_recv().is_err(), "images default off");
    listener.set_images(true);
    std::thread::sleep(Duration::from_millis(40));
    assert!(
        listener.rx.try_recv().is_err(),
        "image activation no catch-up"
    );
    let receive_image = || -> Result<(), String> {
        match listener
            .rx
            .recv_timeout(Duration::from_secs(4))
            .map(|q| q.event)
        {
            Ok(Event::Image(value, _)) if value.sha256 == picture.sha256 => Ok(()),
            _ => Err("isolated PNG image notification mismatch".into()),
        }
    };
    write_picture(&picture, &dib, listener.window as HWND)?;
    receive_image()?;
    let image_reads = IMAGE_READS.load(Ordering::Acquire);
    listener.set_policy(CapturePolicy::parse(&owner, false)?);
    write_picture(&picture, &dib, listener.window as HWND)?;
    assert!(matches!(
        listener
            .rx
            .recv_timeout(Duration::from_secs(2))
            .map(|q| q.event),
        Ok(Event::Excluded)
    ));
    assert_eq!(IMAGE_READS.load(Ordering::Acquire), image_reads);
    listener.set_policy(CapturePolicy::default());
    listener.copy_picture(&picture, &dib)?;
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        listener.rx.try_recv().is_err(),
        "image self-copy suppression"
    );
    write_picture(&picture, &dib, listener.window as HWND)?;
    receive_image()?;
    listener.set_paused(true);
    write_picture(&picture, &dib, listener.window as HWND)?;
    std::thread::sleep(Duration::from_millis(80));
    assert!(listener.rx.try_recv().is_err());
    listener.set_paused(false);
    std::thread::sleep(Duration::from_millis(40));
    assert!(listener.rx.try_recv().is_err());
    let big = Picture::from_image(image::DynamicImage::new_rgba8(2000, 2000))
        .map_err(|_| "large fixture encode")?;
    let big_dib = super::media::to_dib(&big).map_err(|_| "large fixture DIB")?;
    let before = IMAGE_READS.load(Ordering::Acquire);
    write_picture(&big, &big_dib, listener.window as HWND)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while IMAGE_READS.load(Ordering::Acquire) == before && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(IMAGE_READS.load(Ordering::Acquire) > before);
    listener.set_paused(true);
    std::thread::sleep(Duration::from_millis(600));
    assert!(
        listener.rx.try_recv().is_err(),
        "in-flight decoded image cannot arrive after pause"
    );
    listener.set_paused(false);
    std::thread::sleep(Duration::from_millis(80));
    assert!(listener.rx.try_recv().is_err());
    // DIBV5 only tests standard-format decoding rather than the PNG-preferred path.
    unsafe {
        let handle = memory(&dib)?;
        assert_ne!(OpenClipboard(listener.window as HWND), 0);
        let opened = Clipboard;
        assert_ne!(EmptyClipboard(), 0);
        assert!(!SetClipboardData(17, handle).is_null());
        drop(opened);
    }
    receive_image()?;
    println!(
        "PASS isolated clipboard images: PNG transparency, DIBV5-only pixels, default off/no catch-up, pre-read image exclusions, in-flight pause generation and self-copy suppression"
    );
    assert!(write_text("零\0字符", listener.window as HWND).is_err());
    assert!(write_text(&"x".repeat(TEXT_LIMIT + 1), listener.window as HWND).is_err());
    drop(listener);
    println!(
        "PASS isolated Windows clipboard: actual notifications, UTF16 multiline, pause/no catch-up, own-output suppression, same-text external recopy, limits, pre-read process exclusions/no catch-up and shutdown; interactive clipboard untouched"
    );
    Ok(())
}

#[cfg(test)]
mod queue_tests {
    use super::*;
    #[test]
    fn byte_reservations_return_on_consumption_and_channel_rejection() {
        let total = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hold = reserve(&total, BYTE_LIMIT).unwrap();
        assert!(reserve(&total, 1).is_none());
        drop(hold);
        assert_eq!(total.load(Ordering::Acquire), 0);
        let (tx, rx) = bounded(1);
        tx.try_send(Queued {
            event: Event::Excluded,
            _reservation: reserve(&total, 10).unwrap(),
        })
        .ok()
        .unwrap();
        assert!(
            tx.try_send(Queued {
                event: Event::Excluded,
                _reservation: reserve(&total, 20).unwrap()
            })
            .is_err()
        );
        assert_eq!(total.load(Ordering::Acquire), 10);
        drop(rx.recv().unwrap());
        assert_eq!(total.load(Ordering::Acquire), 0);
    }
}
