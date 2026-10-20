//! A message-only window owns the opt-in Windows listener; no active polling.
use super::{
    BYTE_LIMIT, CapturePolicy, FILE_REFERENCE_LIMIT, FILE_REFERENCE_PATH_BYTE_LIMIT, Picture,
    TEXT_LIMIT,
};
#[cfg(feature = "ui-preview")]
static TEXT_READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "ui-preview")]
static IMAGE_READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "ui-preview")]
static FILE_REFERENCE_READS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "ui-preview")]
static HTML_READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "ui-preview")]
static RTF_READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
use crossbeam_channel::{Receiver, bounded};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{DataExchange::*, Memory::*, Threading::*},
    UI::{Shell::DragQueryFileW, WindowsAndMessaging::*},
};

pub enum Event {
    Text(String, String),
    Error(String),
    Excluded,
    Image(Arc<Picture>, String),
    Files(Vec<String>, String),
    RichText(String, String, String),
    RichRtf(String, Vec<u8>, String),
    RichFormats(String, String, Vec<u8>, String),
}
enum Raw {
    Event(Event),
    Image(Vec<u8>, bool, String),
    Files(Vec<String>, String),
    RichText(String, String, String),
    RichRtf(String, Vec<u8>, String),
    RichFormats(String, String, Vec<u8>, String),
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
    files: Arc<std::sync::atomic::AtomicBool>,
    html: Arc<std::sync::atomic::AtomicBool>,
    rtf: Arc<std::sync::atomic::AtomicBool>,
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
        let files = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_files = files.clone();
        let html = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_html = html.clone();
        let rtf = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_rtf = rtf.clone();
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
                                thread_files.load(Ordering::Acquire),
                                thread_html.load(Ordering::Acquire),
                                thread_rtf.load(Ordering::Acquire),
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
                        Some(Raw::Files(paths, source)) => Some(Event::Files(paths, source)),
                        Some(Raw::RichText(text, html, source)) => {
                            Some(Event::RichText(text, html, source))
                        }
                        Some(Raw::RichRtf(text, rtf, source)) => {
                            Some(Event::RichRtf(text, rtf, source))
                        }
                        Some(Raw::RichFormats(text, html, rtf, source)) => {
                            Some(Event::RichFormats(text, html, rtf, source))
                        }
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
                            Event::Files(paths, _) => paths.iter().map(String::len).sum::<usize>(),
                            Event::RichText(text, html, _) => text.len().saturating_add(html.len()),
                            Event::RichRtf(text, rtf, _) => text.len().saturating_add(rtf.len()),
                            Event::RichFormats(text, html, rtf, _) => text
                                .len()
                                .saturating_add(html.len())
                                .saturating_add(rtf.len()),
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
            files,
            html,
            rtf,
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
    pub fn set_files(&self, value: bool) {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.files.store(value, Ordering::Release);
        self.own_sequence
            .store(unsafe { GetClipboardSequenceNumber() }, Ordering::Release);
        while self.rx.try_recv().is_ok() {}
    }
    pub fn set_html(&self, value: bool) {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.html.store(value, Ordering::Release);
        self.own_sequence
            .store(unsafe { GetClipboardSequenceNumber() }, Ordering::Release);
        while self.rx.try_recv().is_ok() {}
    }
    pub fn set_rtf(&self, value: bool) {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.rtf.store(value, Ordering::Release);
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
    pub fn copy_html(&self, text: &str, html: &str) -> Result<(), String> {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        match write_html(text, html, self.window as HWND) {
            Ok(sequence) => {
                self.own_sequence.store(sequence, Ordering::Release);
                Ok(())
            }
            Err((error, clipboard_changed)) => {
                if clipboard_changed {
                    self.own_sequence
                        .store(unsafe { GetClipboardSequenceNumber() }, Ordering::Release);
                }
                Err(error)
            }
        }
    }
    pub fn copy_rtf(&self, text: &str, rtf: &[u8]) -> Result<(), String> {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        match write_rtf(text, rtf, self.window as HWND) {
            Ok(sequence) => {
                self.own_sequence.store(sequence, Ordering::Release);
                Ok(())
            }
            Err((error, clipboard_changed)) => {
                if clipboard_changed {
                    self.own_sequence
                        .store(unsafe { GetClipboardSequenceNumber() }, Ordering::Release);
                }
                Err(error)
            }
        }
    }
}

pub fn copy_html_once(text: &str, html: &str) -> Result<(), String> {
    let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
    let window = unsafe {
        CreateWindowExW(
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
        )
    };
    if window.is_null() {
        return Err("无法创建HTML剪贴板写入窗口".into());
    }
    let result = write_html(text, html, window);
    unsafe { DestroyWindow(window) };
    result.map(|_| ()).map_err(|(error, _)| error)
}
pub fn copy_rtf_once(text: &str, rtf: &[u8]) -> Result<(), String> {
    let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
    let window = unsafe {
        CreateWindowExW(
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
        )
    };
    if window.is_null() {
        return Err("无法创建RTF剪贴板写入窗口".into());
    }
    let result = write_rtf(text, rtf, window);
    unsafe { DestroyWindow(window) };
    result.map(|_| ()).map_err(|(error, _)| error)
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

unsafe fn read_registered_payload(
    format: u32,
    limit: usize,
    label: &str,
) -> Result<Vec<u8>, String> {
    let handle = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return Err(format!("{label}剪贴板格式读取失败"));
    }
    let size = unsafe { GlobalSize(handle) };
    if !(1..=limit + 16 * 1024).contains(&size) {
        return Err(format!("{label}剪贴板数据为空或超过2 MiB上限"));
    }
    let ptr = unsafe { GlobalLock(handle) } as *const u8;
    if ptr.is_null() {
        return Err(format!("无法锁定{label}剪贴板数据"));
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, size) }.to_vec();
    unsafe { GlobalUnlock(handle) };
    Ok(bytes)
}

fn normalize_rtf_payload(mut bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    super::validate_rtf_payload(&bytes)?;
    Ok(bytes)
}

fn rich_formats_from_payload(
    plain_text: Option<String>,
    html_payload: &[u8],
    rtf_payload: Vec<u8>,
    source: String,
) -> Result<Raw, String> {
    let html = parse_cf_html(html_payload)?;
    let rtf = normalize_rtf_payload(rtf_payload)?;
    Ok(Raw::RichFormats(
        plain_text.unwrap_or_default(),
        html,
        rtf,
        source,
    ))
}

unsafe fn read_text(
    sequence: u32,
    policy: &CapturePolicy,
    images: bool,
    files: bool,
    html: bool,
    rtf: bool,
) -> Result<Option<Raw>, String> {
    let png = png_format()?;
    let image_format = if images {
        [png, 17, 8]
            .into_iter()
            .find(|format| unsafe { IsClipboardFormatAvailable(*format) } != 0)
    } else {
        None
    };
    const CF_HDROP_FORMAT: u32 = 15;
    let files_available = unsafe { IsClipboardFormatAvailable(CF_HDROP_FORMAT) } != 0;
    let html_format = if html { html_format()? } else { 0 };
    let html_available =
        html_format != 0 && unsafe { IsClipboardFormatAvailable(html_format) } != 0;
    let rtf_format = if rtf { rtf_format()? } else { 0 };
    let rtf_available = rtf_format != 0 && unsafe { IsClipboardFormatAvailable(rtf_format) } != 0;
    if image_format.is_none() && files_available && !files {
        return Ok(None);
    }
    if image_format.is_none()
        && !files_available
        && !html_available
        && !rtf_available
        && unsafe { IsClipboardFormatAvailable(13) } == 0
    {
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
    if html_available && rtf_available {
        #[cfg(feature = "ui-preview")]
        {
            HTML_READS.fetch_add(1, Ordering::Relaxed);
            RTF_READS.fetch_add(1, Ordering::Relaxed);
        }
        let html_bytes =
            unsafe { read_registered_payload(html_format, super::HTML_LIMIT, "HTML")? };
        let rtf_bytes = unsafe { read_registered_payload(rtf_format, super::RTF_LIMIT, "RTF")? };
        let plain_text = if unsafe { IsClipboardFormatAvailable(13) } != 0 {
            Some(read_plain_text()?)
        } else {
            None
        };
        return rich_formats_from_payload(plain_text, &html_bytes, rtf_bytes, source).map(Some);
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
    if files_available {
        if !files {
            return Ok(None);
        }
        #[cfg(feature = "ui-preview")]
        FILE_REFERENCE_READS.fetch_add(1, Ordering::Relaxed);
        let handle = unsafe { GetClipboardData(CF_HDROP_FORMAT) };
        if handle.is_null() {
            return Err("文件引用读取失败".into());
        }
        let count = unsafe { DragQueryFileW(handle, u32::MAX, std::ptr::null_mut(), 0) };
        if count == 0 || count as usize > FILE_REFERENCE_LIMIT {
            return Err(format!("文件引用数量无效或超过{FILE_REFERENCE_LIMIT}项"));
        }
        let mut paths = Vec::with_capacity(count as usize);
        let mut total_bytes = 0usize;
        for index in 0..count {
            let required = unsafe { DragQueryFileW(handle, index, std::ptr::null_mut(), 0) };
            if required == 0 || required as usize > FILE_REFERENCE_PATH_BYTE_LIMIT / 2 {
                return Err("文件引用路径为空或过长".into());
            }
            let mut buffer = vec![0u16; required as usize + 1];
            let written =
                unsafe { DragQueryFileW(handle, index, buffer.as_mut_ptr(), buffer.len() as u32) };
            if written != required {
                return Err("文件引用路径读取不完整".into());
            }
            let path = String::from_utf16(&buffer[..written as usize])
                .map_err(|_| "文件引用路径UTF-16无效")?;
            if path.len() > FILE_REFERENCE_PATH_BYTE_LIMIT {
                return Err("文件引用路径超过32 KiB".into());
            }
            if path.chars().any(char::is_control) {
                return Err("文件引用路径含控制字符".into());
            }
            total_bytes = total_bytes.saturating_add(path.len());
            if total_bytes > super::FILE_REFERENCE_BYTE_LIMIT {
                return Err("文件引用路径总量超过1 MiB".into());
            }
            paths.push(path);
        }
        return Ok(Some(Raw::Files(paths, source)));
    }
    if html_available {
        #[cfg(feature = "ui-preview")]
        HTML_READS.fetch_add(1, Ordering::Relaxed);
        let bytes = unsafe { read_registered_payload(html_format, super::HTML_LIMIT, "HTML")? };
        let fragment = parse_cf_html(&bytes)?;
        let plain_text = if unsafe { IsClipboardFormatAvailable(13) } != 0 {
            read_plain_text()?
        } else {
            String::new()
        };
        return Ok(Some(Raw::RichText(plain_text, fragment, source)));
    }
    if rtf_available {
        #[cfg(feature = "ui-preview")]
        RTF_READS.fetch_add(1, Ordering::Relaxed);
        let bytes = unsafe { read_registered_payload(rtf_format, super::RTF_LIMIT, "RTF")? };
        let bytes = normalize_rtf_payload(bytes)?;
        let plain_text = if unsafe { IsClipboardFormatAvailable(13) } != 0 {
            read_plain_text()?
        } else {
            String::new()
        };
        return Ok(Some(Raw::RichRtf(plain_text, bytes, source)));
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

fn html_format() -> Result<u32, String> {
    let name: Vec<u16> = "HTML Format\0".encode_utf16().collect();
    let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
    if format == 0 {
        Err("无法注册HTML剪贴板格式".into())
    } else {
        Ok(format)
    }
}

fn rtf_format() -> Result<u32, String> {
    let name: Vec<u16> = "Rich Text Format\0".encode_utf16().collect();
    let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
    if format == 0 {
        Err("无法注册RTF剪贴板格式".into())
    } else {
        Ok(format)
    }
}

fn parse_cf_html(bytes: &[u8]) -> Result<String, String> {
    if bytes.is_empty() || bytes.len() > super::HTML_LIMIT + 16 * 1024 {
        return Err("HTML剪贴板数据为空或超出上限".into());
    }
    let (start_html_value, _) = cf_html_field(bytes, "StartHTML")?;
    let (end_html_value, _) = cf_html_field(bytes, "EndHTML")?;
    let (start_fragment_value, _) = cf_html_field(bytes, "StartFragment")?;
    let (end_fragment_value, header_end) = cf_html_field(bytes, "EndFragment")?;
    let start_html = parse_cf_html_offset(start_html_value, "StartHTML", true)?;
    let end_html = parse_cf_html_offset(end_html_value, "EndHTML", true)?;
    let start = parse_cf_html_offset(start_fragment_value, "StartFragment", false)?
        .ok_or_else(|| "HTML剪贴板StartFragment偏移无效".to_string())?;
    let end = parse_cf_html_offset(end_fragment_value, "EndFragment", false)?
        .ok_or_else(|| "HTML剪贴板EndFragment偏移无效".to_string())?;
    let context_valid = match (start_html, end_html) {
        (Some(start_html), Some(end_html)) => {
            start_html <= start && end <= end_html && end_html <= bytes.len()
        }
        (None, None) => true,
        _ => false,
    };
    if !context_valid
        || header_end > start
        || start >= end
        || end > bytes.len()
        || end - start > super::HTML_LIMIT
    {
        return Err("HTML剪贴板片段偏移越界或超过2 MiB".into());
    }
    let fragment = std::str::from_utf8(&bytes[start..end]).map_err(|_| "HTML片段不是有效UTF-8")?;
    if fragment.contains('\0') || fragment.trim().is_empty() {
        return Err("HTML片段为空或包含零字符".into());
    }
    Ok(fragment.to_string())
}

/// Read a CF_HTML header field and return the byte offset immediately after its line.
/// The format permits CRLF, LF, or CR line endings.
fn cf_html_field<'a>(bytes: &'a [u8], name: &str) -> Result<(&'a [u8], usize), String> {
    let limit = bytes.len().min(16 * 1024);
    let prefix = format!("{name}:").into_bytes();
    let mut cursor = 0;
    while cursor < limit {
        let line_start = cursor;
        while cursor < limit && !matches!(bytes[cursor], b'\r' | b'\n') {
            cursor += 1;
        }
        let line = &bytes[line_start..cursor];
        if cursor < limit {
            if bytes[cursor] == b'\r' && bytes.get(cursor + 1) == Some(&b'\n') {
                cursor += 2;
            } else {
                cursor += 1;
            }
        }
        if let Some(value) = line.strip_prefix(prefix.as_slice()) {
            return Ok((value, cursor));
        }
    }
    Err(format!("HTML剪贴板缺少{name}"))
}

fn parse_cf_html_offset(
    value: &[u8],
    name: &str,
    allow_unknown: bool,
) -> Result<Option<usize>, String> {
    let value = std::str::from_utf8(value)
        .map_err(|_| format!("HTML剪贴板{name}不是ASCII"))?
        .trim();
    if allow_unknown && value == "-1" {
        return Ok(None);
    }
    value
        .parse::<usize>()
        .map(Some)
        .map_err(|_| format!("HTML剪贴板{name}偏移无效"))
}

fn build_cf_html(fragment: &str) -> Result<Vec<u8>, String> {
    if fragment.is_empty() || fragment.len() > super::HTML_LIMIT || fragment.contains('\0') {
        return Err("HTML片段为空、包含零字符或超过2 MiB".into());
    }
    let prefix = "<!DOCTYPE html><html><body><!--StartFragment-->";
    let suffix = "<!--EndFragment--></body></html>";
    let document = format!("{prefix}{fragment}{suffix}");
    let header_template = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    let start_html = header_template.len();
    let start_fragment = start_html + prefix.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = start_html + document.len();
    let header = format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n"
    );
    debug_assert_eq!(header.len(), header_template.len());
    Ok([header.as_bytes(), document.as_bytes()].concat())
}

fn read_plain_text() -> Result<String, String> {
    let handle = unsafe { GetClipboardData(13) };
    if handle.is_null() {
        return Err("HTML剪贴板纯文本读取失败".into());
    }
    let size = unsafe { GlobalSize(handle) };
    if !(2..=TEXT_LIMIT * 2 + 2).contains(&size) || size % 2 != 0 {
        return Err("HTML剪贴板纯文本大小无效或超出上限".into());
    }
    let ptr = unsafe { GlobalLock(handle) } as *const u16;
    if ptr.is_null() {
        return Err("无法锁定HTML剪贴板纯文本".into());
    }
    let words = unsafe { std::slice::from_raw_parts(ptr, size / 2) };
    let decoded = words
        .iter()
        .position(|word| *word == 0)
        .ok_or("HTML剪贴板纯文本未结束")
        .and_then(|end| {
            String::from_utf16(&words[..end]).map_err(|_| "HTML剪贴板纯文本UTF-16无效")
        });
    unsafe { GlobalUnlock(handle) };
    let text = decoded.map_err(str::to_string)?;
    if text.len() > TEXT_LIMIT {
        return Err("HTML剪贴板纯文本超过1 MiB".into());
    }
    Ok(text)
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

fn write_html(text: &str, html: &str, window: HWND) -> Result<u32, (String, bool)> {
    if text.len() > TEXT_LIMIT || text.contains('\0') {
        return Err(("HTML的纯文本回退超限或包含零字符".into(), false));
    }
    let html_format = html_format().map_err(|error| (error, false))?;
    let html_bytes = build_cf_html(html).map_err(|error| (error, false))?;
    let words: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let text_handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, words.len() * 2) };
    if text_handle.is_null() {
        return Err(("纯文本剪贴板内存分配失败".into(), false));
    }
    let text_ptr = unsafe { GlobalLock(text_handle) } as *mut u16;
    if text_ptr.is_null() {
        unsafe { GlobalFree(text_handle) };
        return Err(("纯文本剪贴板内存锁定失败".into(), false));
    }
    unsafe {
        std::ptr::copy_nonoverlapping(words.as_ptr(), text_ptr, words.len());
        GlobalUnlock(text_handle);
    }
    let html_handle = match memory(&html_bytes) {
        Ok(handle) => handle,
        Err(error) => {
            unsafe { GlobalFree(text_handle) };
            return Err((error, false));
        }
    };
    let opened: Result<Clipboard, (String, bool)> = if unsafe { OpenClipboard(window) } != 0 {
        Ok(Clipboard)
    } else {
        Err(("剪贴板被其他程序占用，未复制".into(), false))
    };
    let Ok(opened) = opened else {
        unsafe {
            GlobalFree(text_handle);
            GlobalFree(html_handle);
        }
        return Err(("剪贴板被其他程序占用，未复制".into(), false));
    };
    if unsafe { EmptyClipboard() } == 0 {
        unsafe {
            GlobalFree(text_handle);
            GlobalFree(html_handle);
        }
        return Err(("无法重置剪贴板，未复制".into(), false));
    }
    if unsafe { SetClipboardData(13, text_handle) }.is_null() {
        unsafe {
            GlobalFree(text_handle);
            GlobalFree(html_handle);
        }
        return Err(("纯文本剪贴板写入失败".into(), true));
    }
    if unsafe { SetClipboardData(html_format, html_handle) }.is_null() {
        unsafe { GlobalFree(html_handle) };
        return Err(("纯文本已复制，但HTML格式写入失败".into(), true));
    }
    drop(opened);
    Ok(unsafe { GetClipboardSequenceNumber() })
}

fn write_rtf(text: &str, rtf: &[u8], window: HWND) -> Result<u32, (String, bool)> {
    if text.len() > TEXT_LIMIT || text.contains('\0') {
        return Err(("RTF的纯文本回退超限或包含零字符".into(), false));
    }
    super::validate_rtf_payload(rtf).map_err(|error| (error, false))?;
    let format = rtf_format().map_err(|error| (error, false))?;
    let words: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let text_handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, words.len() * 2) };
    if text_handle.is_null() {
        return Err(("RTF纯文本剪贴板内存分配失败".into(), false));
    }
    let text_ptr = unsafe { GlobalLock(text_handle) } as *mut u16;
    if text_ptr.is_null() {
        unsafe { GlobalFree(text_handle) };
        return Err(("RTF纯文本剪贴板内存锁定失败".into(), false));
    }
    unsafe {
        std::ptr::copy_nonoverlapping(words.as_ptr(), text_ptr, words.len());
        GlobalUnlock(text_handle);
    }
    // The registered Windows RTF format is conventionally NUL-terminated.
    // Store the content without terminators and add exactly one when publishing it.
    let mut clipboard_rtf = Vec::with_capacity(rtf.len() + 1);
    clipboard_rtf.extend_from_slice(rtf);
    clipboard_rtf.push(0);
    let rtf_handle = match memory(&clipboard_rtf) {
        Ok(handle) => handle,
        Err(error) => {
            unsafe { GlobalFree(text_handle) };
            return Err((error, false));
        }
    };
    let opened = if unsafe { OpenClipboard(window) } != 0 {
        Ok(Clipboard)
    } else {
        Err(("剪贴板被其他程序占用，未复制".into(), false))
    };
    let opened = match opened {
        Ok(opened) => opened,
        Err(error) => {
            unsafe {
                GlobalFree(text_handle);
                GlobalFree(rtf_handle);
            }
            return Err(error);
        }
    };
    if unsafe { EmptyClipboard() } == 0 {
        unsafe {
            GlobalFree(text_handle);
            GlobalFree(rtf_handle);
        }
        return Err(("无法重置剪贴板，未复制".into(), false));
    }
    if unsafe { SetClipboardData(13, text_handle) }.is_null() {
        unsafe {
            GlobalFree(text_handle);
            GlobalFree(rtf_handle);
        }
        return Err(("RTF纯文本剪贴板写入失败".into(), true));
    }
    if unsafe { SetClipboardData(format, rtf_handle) }.is_null() {
        unsafe { GlobalFree(rtf_handle) };
        return Err(("纯文本已复制，但RTF格式写入失败".into(), true));
    }
    drop(opened);
    Ok(unsafe { GetClipboardSequenceNumber() })
}

#[cfg(feature = "ui-preview")]
fn write_file_references(paths: &[&str], window: HWND) -> Result<u32, String> {
    let mut bytes = vec![0u8; 20];
    bytes[..4].copy_from_slice(&20u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&1i32.to_le_bytes());
    for path in paths {
        bytes.extend(path.encode_utf16().flat_map(u16::to_le_bytes));
        bytes.extend([0, 0]);
    }
    bytes.extend([0, 0]);
    let handle = memory(&bytes)?;
    unsafe {
        if OpenClipboard(window) == 0 {
            GlobalFree(handle);
            return Err("隔离剪贴板正被占用".into());
        }
        let opened = Clipboard;
        if EmptyClipboard() == 0 {
            GlobalFree(handle);
            return Err("无法重置隔离剪贴板".into());
        }
        const CF_HDROP_FORMAT: u32 = 15;
        if SetClipboardData(CF_HDROP_FORMAT, handle).is_null() {
            GlobalFree(handle);
            return Err("无法写入隔离文件引用".into());
        }
        drop(opened);
        Ok(GetClipboardSequenceNumber())
    }
}

#[cfg(feature = "ui-preview")]
fn write_html_fixture(text: &str, html: &str, window: HWND) -> Result<u32, String> {
    let html_format = html_format()?;
    let html_bytes = build_cf_html(html)?;
    let text_words: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let mut text_bytes = Vec::with_capacity(text_words.len() * 2);
    for word in text_words {
        text_bytes.extend_from_slice(&word.to_le_bytes());
    }
    let text_handle = memory(&text_bytes)?;
    let html_handle = match memory(&html_bytes) {
        Ok(handle) => handle,
        Err(error) => {
            unsafe { GlobalFree(text_handle) };
            return Err(error);
        }
    };
    unsafe {
        if OpenClipboard(window) == 0 {
            GlobalFree(text_handle);
            GlobalFree(html_handle);
            return Err("隔离剪贴板正被占用".into());
        }
        let opened = Clipboard;
        if EmptyClipboard() == 0 {
            GlobalFree(text_handle);
            GlobalFree(html_handle);
            return Err("无法重置隔离剪贴板".into());
        }
        if SetClipboardData(13, text_handle).is_null() {
            GlobalFree(text_handle);
            GlobalFree(html_handle);
            return Err("隔离纯文本写入失败".into());
        }
        if SetClipboardData(html_format, html_handle).is_null() {
            GlobalFree(html_handle);
            return Err("隔离HTML写入失败".into());
        }
        drop(opened);
        Ok(GetClipboardSequenceNumber())
    }
}

#[cfg(feature = "ui-preview")]
fn write_rtf_fixture(text: &str, rtf: &[u8], window: HWND) -> Result<u32, String> {
    let format = rtf_format()?;
    super::validate_rtf_payload(rtf)?;
    let text_words: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let mut text_bytes = Vec::with_capacity(text_words.len() * 2);
    for word in text_words {
        text_bytes.extend_from_slice(&word.to_le_bytes());
    }
    let text_handle = memory(&text_bytes)?;
    let mut clipboard_rtf = Vec::with_capacity(rtf.len() + 1);
    clipboard_rtf.extend_from_slice(rtf);
    clipboard_rtf.push(0);
    let rtf_handle = match memory(&clipboard_rtf) {
        Ok(handle) => handle,
        Err(error) => {
            unsafe { GlobalFree(text_handle) };
            return Err(error);
        }
    };
    unsafe {
        if OpenClipboard(window) == 0 {
            GlobalFree(text_handle);
            GlobalFree(rtf_handle);
            return Err("隔离剪贴板正被占用".into());
        }
        let opened = Clipboard;
        if EmptyClipboard() == 0 {
            GlobalFree(text_handle);
            GlobalFree(rtf_handle);
            return Err("无法重置隔离剪贴板".into());
        }
        if SetClipboardData(13, text_handle).is_null() {
            GlobalFree(text_handle);
            GlobalFree(rtf_handle);
            return Err("隔离纯文本写入失败".into());
        }
        if SetClipboardData(format, rtf_handle).is_null() {
            GlobalFree(rtf_handle);
            return Err("隔离RTF写入失败".into());
        }
        drop(opened);
        Ok(GetClipboardSequenceNumber())
    }
}

#[cfg(feature = "ui-preview")]
fn write_html_rtf_fixture(text: &str, html: &str, rtf: &[u8], window: HWND) -> Result<u32, String> {
    let html_format = html_format()?;
    let rtf_format = rtf_format()?;
    let html_bytes = build_cf_html(html)?;
    super::validate_rtf_payload(rtf)?;
    let text_words: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let mut text_bytes = Vec::with_capacity(text_words.len() * 2);
    for word in text_words {
        text_bytes.extend_from_slice(&word.to_le_bytes());
    }
    let text_handle = memory(&text_bytes)?;
    let html_handle = match memory(&html_bytes) {
        Ok(handle) => handle,
        Err(error) => {
            unsafe { GlobalFree(text_handle) };
            return Err(error);
        }
    };
    let mut clipboard_rtf = Vec::with_capacity(rtf.len() + 1);
    clipboard_rtf.extend_from_slice(rtf);
    clipboard_rtf.push(0);
    let rtf_handle = match memory(&clipboard_rtf) {
        Ok(handle) => handle,
        Err(error) => {
            unsafe {
                GlobalFree(text_handle);
                GlobalFree(html_handle);
            }
            return Err(error);
        }
    };
    unsafe {
        if OpenClipboard(window) == 0 {
            GlobalFree(text_handle);
            GlobalFree(html_handle);
            GlobalFree(rtf_handle);
            return Err("隔离剪贴板正被占用".into());
        }
        let opened = Clipboard;
        if EmptyClipboard() == 0 {
            GlobalFree(text_handle);
            GlobalFree(html_handle);
            GlobalFree(rtf_handle);
            return Err("无法重置隔离剪贴板".into());
        }
        if SetClipboardData(13, text_handle).is_null() {
            GlobalFree(text_handle);
            GlobalFree(html_handle);
            GlobalFree(rtf_handle);
            return Err("隔离纯文本写入失败".into());
        }
        if SetClipboardData(html_format, html_handle).is_null() {
            GlobalFree(html_handle);
            GlobalFree(rtf_handle);
            return Err("隔离HTML写入失败".into());
        }
        if SetClipboardData(rtf_format, rtf_handle).is_null() {
            GlobalFree(rtf_handle);
            return Err("隔离RTF写入失败".into());
        }
        drop(opened);
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
    use windows_sys::Win32::Security::{
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
    };
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
    let mut station = unsafe {
        let old = GetProcessWindowStation();
        let old_desktop = GetThreadDesktop(GetCurrentThreadId());
        // An explicit random name gives the fixture a distinct clipboard. Windows only lets
        // Administrators create named stations; CREATE_ONLY prevents accidental reuse.
        let name = format!("ZiDevToolsClipboardFixture-{}", uuid::Uuid::new_v4());
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let security_sddl: Vec<u16> = "D:P(A;;GA;;;OW)\0".encode_utf16().collect();
        let mut security_descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            security_sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut security_descriptor,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(format!(
                "Private station security descriptor creation failed ({}); no clipboard access",
                GetLastError()
            ));
        }
        let security_attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: security_descriptor,
            bInheritHandle: 0,
        };
        let station_access = (WINSTA_ACCESSCLIPBOARD | WINSTA_CREATEDESKTOP) as u32;
        let handle = CreateWindowStationW(
            wide.as_ptr(),
            CWF_CREATE_ONLY,
            station_access,
            &security_attributes,
        );
        LocalFree(security_descriptor as HLOCAL);
        if handle.is_null() {
            let error = GetLastError();
            let detail = if error == ERROR_ACCESS_DENIED {
                "；请从提升权限的管理员终端运行隔离夹具"
            } else if error == ERROR_ALREADY_EXISTS {
                "；系统生成的窗口站已存在，已拒绝复用"
            } else {
                ""
            };
            return Err(format!(
                "Private window station creation failed ({error}){detail}; no clipboard access"
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
        let desktop_access = DESKTOP_CREATEWINDOW | DESKTOP_READOBJECTS | DESKTOP_WRITEOBJECTS;
        CreateDesktopW(
            desktop_name.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            desktop_access,
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
    let html_before = HTML_READS.load(Ordering::Acquire);
    write_html_fixture(
        "纯文本回退",
        "<p><strong>隔离格式</strong></p>",
        listener.window as HWND,
    )?;
    match listener
        .rx
        .recv_timeout(Duration::from_secs(2))
        .map(|queued| queued.event)
    {
        Ok(Event::Text(text, _)) if text == "纯文本回退" => {}
        _ => return Err("HTML默认关闭时应仅读取纯文本回退".into()),
    }
    assert_eq!(
        HTML_READS.load(Ordering::Acquire),
        html_before,
        "HTML data is not requested while disabled"
    );
    listener.set_html(true);
    std::thread::sleep(Duration::from_millis(40));
    assert!(
        listener.rx.try_recv().is_err(),
        "HTML activation no catch-up"
    );
    write_html_fixture(
        "纯文本回退",
        "<p><strong>隔离格式</strong></p>",
        listener.window as HWND,
    )?;
    match listener
        .rx
        .recv_timeout(Duration::from_secs(2))
        .map(|queued| queued.event)
    {
        Ok(Event::RichText(text, html, _))
            if text == "纯文本回退" && html == "<p><strong>隔离格式</strong></p>" => {}
        _ => return Err("isolated clipboard CF_HTML fragment mismatch".into()),
    }
    assert!(HTML_READS.load(Ordering::Acquire) > html_before);
    listener.copy_html("工具输出", "<p>工具<strong>输出</strong></p>")?;
    std::thread::sleep(Duration::from_millis(80));
    assert!(
        listener.rx.try_recv().is_err(),
        "HTML self-copy suppression"
    );
    listener.set_policy(CapturePolicy::parse(&owner, false)?);
    let excluded_html_before = HTML_READS.load(Ordering::Acquire);
    write_html_fixture("排除纯文本", "<p>排除HTML</p>", listener.window as HWND)?;
    assert!(matches!(
        listener
            .rx
            .recv_timeout(Duration::from_secs(2))
            .map(|queued| queued.event),
        Ok(Event::Excluded)
    ));
    assert_eq!(
        HTML_READS.load(Ordering::Acquire),
        excluded_html_before,
        "excluded source never retrieves HTML data"
    );
    listener.set_policy(CapturePolicy::default());
    listener.set_html(false);
    const RTF_SAMPLE: &[u8] = br#"{\rtf1\ansi Hello {\b world}}"#;
    listener.set_policy(CapturePolicy::parse(&owner, false)?);
    let excluded_rtf_before = RTF_READS.load(Ordering::Acquire);
    write_rtf_fixture("排除纯文本", RTF_SAMPLE, listener.window as HWND)?;
    assert!(matches!(
        listener
            .rx
            .recv_timeout(Duration::from_secs(2))
            .map(|queued| queued.event),
        Ok(Event::Excluded)
    ));
    assert_eq!(
        RTF_READS.load(Ordering::Acquire),
        excluded_rtf_before,
        "excluded process never retrieves RTF data"
    );
    listener.set_policy(CapturePolicy::default());
    let rtf_before = RTF_READS.load(Ordering::Acquire);
    write_rtf_fixture("纯文本回退", RTF_SAMPLE, listener.window as HWND)?;
    match listener
        .rx
        .recv_timeout(Duration::from_secs(2))
        .map(|queued| queued.event)
    {
        Ok(Event::Text(text, _)) if text == "纯文本回退" => {}
        _ => return Err("RTF默认关闭时应仅读取纯文本回退".into()),
    }
    assert_eq!(
        RTF_READS.load(Ordering::Acquire),
        rtf_before,
        "RTF data is not requested while disabled"
    );
    listener.set_rtf(true);
    std::thread::sleep(Duration::from_millis(40));
    assert!(
        listener.rx.try_recv().is_err(),
        "RTF activation no catch-up"
    );
    write_rtf_fixture("纯文本回退", RTF_SAMPLE, listener.window as HWND)?;
    match listener
        .rx
        .recv_timeout(Duration::from_secs(2))
        .map(|queued| queued.event)
    {
        Ok(Event::RichRtf(text, rtf, _)) if text == "纯文本回退" && rtf == RTF_SAMPLE => {}
        _ => return Err("isolated clipboard RTF bytes/fallback mismatch".into()),
    }
    assert!(RTF_READS.load(Ordering::Acquire) > rtf_before);
    listener.copy_rtf("工具输出", RTF_SAMPLE)?;
    std::thread::sleep(Duration::from_millis(80));
    assert!(listener.rx.try_recv().is_err(), "RTF self-copy suppression");
    write_rtf_fixture("纯文本回退", RTF_SAMPLE, listener.window as HWND)?;
    match listener
        .rx
        .recv_timeout(Duration::from_secs(2))
        .map(|queued| queued.event)
    {
        Ok(Event::RichRtf(text, rtf, _)) if text == "纯文本回退" && rtf == RTF_SAMPLE => {}
        _ => return Err("external RTF recopy notification mismatch".into()),
    }
    listener.set_html(true);
    std::thread::sleep(Duration::from_millis(40));
    assert!(
        listener.rx.try_recv().is_err(),
        "HTML activation no catch-up"
    );
    let html_and_rtf_before = (
        HTML_READS.load(Ordering::Acquire),
        RTF_READS.load(Ordering::Acquire),
    );
    write_html_rtf_fixture(
        "双格式回退",
        "<p>HTML <strong>格式</strong></p>",
        RTF_SAMPLE,
        listener.window as HWND,
    )?;
    match listener
        .rx
        .recv_timeout(Duration::from_secs(2))
        .map(|queued| queued.event)
    {
        Ok(Event::RichFormats(text, html, rtf, _))
            if text == "双格式回退"
                && html == "<p>HTML <strong>格式</strong></p>"
                && rtf == RTF_SAMPLE => {}
        _ => return Err("isolated clipboard should preserve HTML and RTF together".into()),
    }
    assert!(HTML_READS.load(Ordering::Acquire) > html_and_rtf_before.0);
    assert!(RTF_READS.load(Ordering::Acquire) > html_and_rtf_before.1);
    listener.set_html(false);
    listener.set_rtf(false);
    let disabled_rtf_before = RTF_READS.load(Ordering::Acquire);
    write_rtf_fixture("关闭后纯文本", RTF_SAMPLE, listener.window as HWND)?;
    match listener
        .rx
        .recv_timeout(Duration::from_secs(2))
        .map(|queued| queued.event)
    {
        Ok(Event::Text(text, _)) if text == "关闭后纯文本" => {}
        _ => return Err("disabled RTF should fall back to plain text".into()),
    }
    assert_eq!(
        RTF_READS.load(Ordering::Acquire),
        disabled_rtf_before,
        "disabling RTF stops reading RTF data"
    );
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
    let screenshot = image::RgbaImage::from_fn(2, 2, |x, y| {
        image::Rgba([x as u8 * 80, y as u8 * 80, 45, if x == y { 0 } else { 192 }])
    });
    let (screenshot_picture, _) =
        super::prepare_screenshot_picture(&screenshot).map_err(|_| "screenshot clipboard prep")?;
    super::copy_screenshot_to_clipboard(&screenshot)?;
    match listener
        .rx
        .recv_timeout(Duration::from_secs(4))
        .map(|q| q.event)
    {
        Ok(Event::Image(value, _)) if value.sha256 == screenshot_picture.sha256 => {}
        _ => return Err("screenshot copy did not publish the expected transparent PNG".into()),
    }
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
    let files_before = FILE_REFERENCE_READS.load(Ordering::Acquire);
    write_file_references(&[r"C:\synthetic\教程.pdf"], listener.window as HWND)?;
    std::thread::sleep(Duration::from_millis(80));
    assert!(
        listener.rx.try_recv().is_err(),
        "file references default off"
    );
    assert_eq!(
        FILE_REFERENCE_READS.load(Ordering::Acquire),
        files_before,
        "disabled references never request CF_HDROP"
    );
    listener.set_files(true);
    std::thread::sleep(Duration::from_millis(40));
    assert!(
        listener.rx.try_recv().is_err(),
        "file activation no catch-up"
    );
    write_file_references(
        &[r"C:\synthetic\教程.pdf", r"C:\synthetic\data.csv"],
        listener.window as HWND,
    )?;
    match listener
        .rx
        .recv_timeout(Duration::from_secs(2))
        .map(|queued| queued.event)
    {
        Ok(Event::Files(paths, _))
            if paths
                == [
                    r"C:\synthetic\教程.pdf".to_string(),
                    r"C:\synthetic\data.csv".to_string(),
                ] => {}
        _ => return Err("isolated clipboard file-reference notification mismatch".into()),
    }
    assert!(FILE_REFERENCE_READS.load(Ordering::Acquire) > files_before);
    listener.set_policy(CapturePolicy::parse(&owner, false)?);
    let excluded_before = FILE_REFERENCE_READS.load(Ordering::Acquire);
    write_file_references(&[r"C:\synthetic\excluded.pdf"], listener.window as HWND)?;
    assert!(matches!(
        listener
            .rx
            .recv_timeout(Duration::from_secs(2))
            .map(|queued| queued.event),
        Ok(Event::Excluded)
    ));
    assert_eq!(
        FILE_REFERENCE_READS.load(Ordering::Acquire),
        excluded_before,
        "excluded process paths never enter the CF_HDROP reader"
    );
    listener.set_policy(CapturePolicy::default());
    listener.set_files(false);
    let disabled_before = FILE_REFERENCE_READS.load(Ordering::Acquire);
    write_file_references(&[r"C:\synthetic\disabled.pdf"], listener.window as HWND)?;
    std::thread::sleep(Duration::from_millis(80));
    assert!(listener.rx.try_recv().is_err());
    assert_eq!(
        FILE_REFERENCE_READS.load(Ordering::Acquire),
        disabled_before,
        "disabling file references stops reading paths"
    );
    println!(
        "PASS isolated clipboard images, HTML and file references: bounded CF_HTML/CF_HDROP data, default off/no catch-up, pre-read source exclusion, in-flight pause generation and self-copy suppression"
    );
    assert!(write_text("零\0字符", listener.window as HWND).is_err());
    assert!(write_text(&"x".repeat(TEXT_LIMIT + 1), listener.window as HWND).is_err());
    drop(listener);
    println!(
        "PASS isolated Windows clipboard: UTF16 and CF_HTML notifications, pause/no catch-up, own-output suppression, same-content external recopy, size limits, pre-read exclusions and shutdown; interactive clipboard untouched"
    );
    Ok(())
}

#[cfg(test)]
mod queue_tests {
    use super::*;

    #[test]
    fn same_clipboard_payload_keeps_both_rich_formats_and_optional_text() {
        let html = build_cf_html("<p>同源格式</p>").unwrap();
        let rtf = br#"{\rtf1\ansi rich}"#.to_vec();
        let source = "fixture.exe".to_string();

        let Raw::RichFormats(text, fragment, actual_rtf, actual_source) =
            rich_formats_from_payload(
                Some("纯文本回退".into()),
                &html,
                [rtf.as_slice(), &[0, 0]].concat(),
                source.clone(),
            )
            .unwrap()
        else {
            panic!("both formats should be emitted as one rich event");
        };
        assert_eq!(text, "纯文本回退");
        assert_eq!(fragment, "<p>同源格式</p>");
        assert_eq!(actual_rtf, rtf);
        assert_eq!(actual_source, source);

        let Raw::RichFormats(text, fragment, actual_rtf, _) =
            rich_formats_from_payload(None, &html, rtf.clone(), "fixture.exe".into()).unwrap()
        else {
            panic!("RTF and HTML should remain available without a plain fallback");
        };
        assert!(text.is_empty());
        assert_eq!(fragment, "<p>同源格式</p>");
        assert_eq!(actual_rtf, rtf);
    }

    #[test]
    fn malformed_member_rejects_the_entire_dual_format_clipboard_payload() {
        let html = build_cf_html("<p>valid html</p>").unwrap();
        assert!(
            rich_formats_from_payload(
                Some("fallback".into()),
                &html,
                b"not rtf".to_vec(),
                "fixture.exe".into(),
            )
            .is_err()
        );
        assert!(
            rich_formats_from_payload(
                Some("fallback".into()),
                b"not CF_HTML",
                br#"{\rtf1 valid}"#.to_vec(),
                "fixture.exe".into(),
            )
            .is_err()
        );
    }

    #[test]
    fn rtf_terminator_normalization_is_bounded_and_validated() {
        let valid = br#"{\rtf1 valid}"#;
        assert_eq!(
            normalize_rtf_payload([valid.as_slice(), &[0, 0]].concat()).unwrap(),
            valid
        );
        assert!(normalize_rtf_payload(vec![0]).is_err());
        assert!(normalize_rtf_payload(vec![b'{'; super::super::RTF_LIMIT + 1]).is_err());
    }

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

    #[test]
    fn cf_html_offsets_are_byte_based_bounded_and_strict() {
        let fragment = "<p>你好 <strong>world</strong></p>";
        let encoded = build_cf_html(fragment).unwrap();
        assert_eq!(parse_cf_html(&encoded).unwrap(), fragment);

        let fragment_only = "<p>片段独立存储</p>";
        for newline in ["\r\n", "\n", "\r"] {
            let template = format!(
                "Version:1.0{newline}StartHTML:-1{newline}EndHTML:-1{newline}StartFragment:0000000000{newline}EndFragment:0000000000{newline}"
            );
            let start = template.len();
            let end = start + fragment_only.len();
            let header = format!(
                "Version:1.0{newline}StartHTML:-1{newline}EndHTML:-1{newline}StartFragment:{start:010}{newline}EndFragment:{end:010}{newline}"
            );
            assert_eq!(header.len(), template.len());
            assert_eq!(
                parse_cf_html(&[header.as_bytes(), fragment_only.as_bytes()].concat()).unwrap(),
                fragment_only,
                "fragment-only CF_HTML permits -1 context offsets and {newline:?} separators"
            );
        }
        let template = "Version:1.0\rStartHTML:-1\rEndHTML:0000000000\rStartFragment:0000000000\rEndFragment:0000000000\r";
        let start = template.len();
        let end = start + fragment_only.len();
        let mismatched_context = format!(
            "Version:1.0\rStartHTML:-1\rEndHTML:{end:010}\rStartFragment:{start:010}\rEndFragment:{end:010}\r{fragment_only}"
        );
        assert!(parse_cf_html(mismatched_context.as_bytes()).is_err());
        assert!(
            parse_cf_html(
                b"Version:1.0\rStartHTML:-1\rEndHTML:-1\rStartFragment:0000000000\rEndFragment:0000000001\rfragment"
            )
            .is_err(),
            "fragment offsets may not point into the header"
        );
        assert!(build_cf_html(&"x".repeat(super::super::HTML_LIMIT + 1)).is_err());
        assert!(
            parse_cf_html(
                b"Version:0.9\r\nStartFragment:0000000040\r\nEndFragment:0000000099\r\nfragment"
            )
            .is_err()
        );
        assert!(parse_cf_html(b"not a CF_HTML document").is_err());
    }
}
