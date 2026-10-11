"""Bounded native MSI dialog probe, exclusively for disposable hosted runners."""
import ctypes
from ctypes import wintypes as w
import os
import struct
import threading
import time
import zlib

TITLE = 'Zi DevTools 安装选项'
LABELS = {'inherit': '保留当前设置（首次安装不启用）', 'on': '启用右键菜单', 'off': '关闭此安装的右键菜单'}


def verify_tables(package):
    api = ctypes.WinDLL('msi')
    for name, params in [
        ('MsiOpenDatabaseW', [w.LPCWSTR, ctypes.c_void_p, ctypes.POINTER(w.UINT)]),
        ('MsiDatabaseOpenViewW', [w.UINT, w.LPCWSTR, ctypes.POINTER(w.UINT)]),
        ('MsiViewExecute', [w.UINT, w.UINT]),
        ('MsiViewFetch', [w.UINT, ctypes.POINTER(w.UINT)]),
        ('MsiRecordGetStringW', [w.UINT, w.UINT, w.LPWSTR, ctypes.POINTER(w.DWORD)]),
        ('MsiCloseHandle', [w.UINT])]:
        getattr(api, name).argtypes = params
        getattr(api, name).restype = w.UINT
    def rows(sql, columns):
        db, view = w.UINT(), w.UINT()
        assert api.MsiOpenDatabaseW(str(package), None, ctypes.byref(db)) == 0
        try:
            assert api.MsiDatabaseOpenViewW(db, sql, ctypes.byref(view)) == 0
            assert api.MsiViewExecute(view, 0) == 0
            result = []
            while True:
                record = w.UINT()
                status = api.MsiViewFetch(view, ctypes.byref(record))
                if status == 259:
                    return result
                assert status == 0 and len(result) < 128
                try:
                    values = []
                    for index in range(1, columns + 1):
                        value, size = ctypes.create_unicode_buffer(4096), w.DWORD(4096)
                        assert api.MsiRecordGetStringW(record, index, value, ctypes.byref(size)) == 0
                        values.append(value.value)
                    result.append(values)
                finally:
                    api.MsiCloseHandle(record)
        finally:
            if view.value:
                api.MsiCloseHandle(view)
            api.MsiCloseHandle(db)
    assert rows("SELECT `Value` FROM `Property` WHERE `Property`='ZIMENUCHOICE'", 1) == [['inherit']]
    radios = rows("SELECT `Value`,`Text` FROM `RadioButton` WHERE `Property`='ZIMENUCHOICE' ORDER BY `Order`", 2)
    assert radios == [[name, LABELS[name]] for name in ['inherit', 'on', 'off']]
    events = rows("SELECT `Event`,`Argument`,`Condition` FROM `ControlEvent` WHERE `Dialog_`='ZiMenuOptions' AND `Control_`='Continue' ORDER BY `Ordering`", 3)
    assert events == [['[ZIMENU]', '1', 'ZIMENUCHOICE = "on"'], ['[ZIMENU]', '0', 'ZIMENUCHOICE = "off"'],
                      ['[ZIMENU]', '{}', 'ZIMENUCHOICE = "inherit"'], ['EndDialog', 'Return', '1']]
    assert rows("SELECT `Event`,`Argument` FROM `ControlEvent` WHERE `Dialog_`='ZiMenuOptions' AND `Control_`='Cancel'", 2) == [['EndDialog', 'Exit']]
    condition = rows("SELECT `Condition` FROM `InstallUISequence` WHERE `Action`='ZiMenuOptions'", 1)
    assert condition == [['UILevel >= 5 AND NOT (REMOVE = "ALL") AND (NOT ZIMENU OR ZIMENU = "0" OR ZIMENU = "1")']]
    return dict(default='inherit', radioValues=[item[0] for item in radios], continueEvents=events,
                showCondition=condition[0][0], cancelEvent='Exit')


def snapshot(hwnd, path, user):
    gdi = ctypes.WinDLL('gdi32', use_last_error=True)
    for name, params, result in [
        ('CreateCompatibleDC', [w.HDC], w.HDC),
        ('CreateDIBSection', [w.HDC, ctypes.c_void_p, w.UINT, ctypes.POINTER(ctypes.c_void_p), w.HANDLE, w.DWORD], w.HANDLE),
        ('SelectObject', [w.HDC, w.HANDLE], w.HANDLE),
        ('DeleteObject', [w.HANDLE], w.BOOL), ('DeleteDC', [w.HDC], w.BOOL)]:
        getattr(gdi, name).argtypes = params
        getattr(gdi, name).restype = result
    user.GetWindowRect.argtypes = [w.HWND, ctypes.POINTER(w.RECT)]
    user.GetWindowRect.restype = w.BOOL
    user.GetDC.argtypes = [w.HWND]; user.GetDC.restype = w.HDC
    user.ReleaseDC.argtypes = [w.HWND, w.HDC]; user.ReleaseDC.restype = ctypes.c_int
    user.PrintWindow.argtypes = [w.HWND, w.HDC, w.UINT]; user.PrintWindow.restype = w.BOOL
    rect = w.RECT()
    assert user.GetWindowRect(hwnd, ctypes.byref(rect))
    width, height = rect.right - rect.left, rect.bottom - rect.top
    assert 200 <= width <= 1800 and 150 <= height <= 1400
    header = ctypes.create_string_buffer(struct.pack('<IiiHHIIiiII', 40, width, -height, 1, 32, 0, width * height * 4, 0, 0, 0, 0))
    screen = user.GetDC(hwnd)
    assert screen
    dc = gdi.CreateCompatibleDC(screen)
    assert dc
    bitmap = previous = None
    try:
        bits = ctypes.c_void_p()
        bitmap = gdi.CreateDIBSection(screen, header, 0, ctypes.byref(bits), None, 0)
        assert bitmap and bits.value
        previous = gdi.SelectObject(dc, bitmap)
        assert previous and user.PrintWindow(hwnd, dc, 0)
        bgra = ctypes.string_at(bits, width * height * 4)
        rgb = bytearray(width * height * 3)
        rgb[0::3], rgb[1::3], rgb[2::3] = bgra[2::4], bgra[1::4], bgra[0::4]
        rows = b''.join(b'\0' + rgb[y * width * 3:(y + 1) * width * 3] for y in range(height))
        def chunk(kind, value):
            return struct.pack('>I', len(value)) + kind + value + struct.pack('>I', zlib.crc32(kind + value))
        path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
                         + chunk(b'IDAT', zlib.compress(rows, 9)) + chunk(b'IEND', b''))
    finally:
        if previous:
            gdi.SelectObject(dc, previous)
        if bitmap:
            gdi.DeleteObject(bitmap)
        gdi.DeleteDC(dc)
        user.ReleaseDC(hwnd, screen)


def click(operation, choice, initial, screenshot, cancel=False, expect_options=True):
    assert os.environ.get('GITHUB_ACTIONS') == 'true' and os.environ.get('CI') == 'true'
    assert os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted'
    user = ctypes.WinDLL('user32', use_last_error=True)
    callback = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    for name, params, result in [
        ('EnumWindows', [callback, w.LPARAM], w.BOOL),
        ('EnumChildWindows', [w.HWND, callback, w.LPARAM], w.BOOL),
        ('GetWindowTextW', [w.HWND, w.LPWSTR, ctypes.c_int], ctypes.c_int),
        ('GetClassNameW', [w.HWND, w.LPWSTR, ctypes.c_int], ctypes.c_int),
        ('GetWindowThreadProcessId', [w.HWND, ctypes.POINTER(w.DWORD)], w.DWORD),
        ('IsWindowVisible', [w.HWND], w.BOOL),
        ('SendMessageW', [w.HWND, w.UINT, w.WPARAM, w.LPARAM], w.LPARAM),
        ('PostMessageW', [w.HWND, w.UINT, w.WPARAM, w.LPARAM], w.BOOL)]:
        getattr(user, name).argtypes = params
        getattr(user, name).restype = result
    def text(hwnd):
        value = ctypes.create_unicode_buffer(1024)
        user.GetWindowTextW(hwnd, value, len(value))
        return value.value
    outcome = []
    def work():
        try:
            outcome.append((True, operation()))
        except BaseException as error:
            outcome.append((False, error))
    worker = threading.Thread(target=work, daemon=True)
    worker.start()
    def finish():
        worker.join(timeout=240)
        assert not worker.is_alive(), 'Native option operation timeout'
        assert len(outcome) == 1
        if not outcome[0][0]:
            raise outcome[0][1]
        return outcome[0][1]
    hwnd = None
    try:
        deadline = time.monotonic() + (40 if expect_options else 240)
        while not hwnd:
            matches = []
            @callback
            def find(candidate, _):
                pid = w.DWORD()
                user.GetWindowThreadProcessId(candidate, ctypes.byref(pid))
                if pid.value == os.getpid() and user.IsWindowVisible(candidate) and text(candidate) == TITLE:
                    matches.append(candidate)
                return True
            assert user.EnumWindows(find, 0)
            assert len(matches) <= 1
            hwnd = matches[0] if matches else None
            if not expect_options:
                assert hwnd is None, 'Uninstall unexpectedly displayed menu options'
                if not worker.is_alive():
                    return finish()
            if not hwnd:
                assert not expect_options or worker.is_alive(), 'Native installation returned before option window'
                assert time.monotonic() < deadline, 'Native option window timeout'
                time.sleep(.1)
        time.sleep(.3)
        children = {}
        invalid_controls = []
        @callback
        def collect(child, _):
            label = text(child)
            if label in list(LABELS.values()) + ['开始安装', '取消']:
                name = ctypes.create_unicode_buffer(128)
                user.GetClassNameW(child, name, len(name))
                if name.value.lower() != 'button' or label in children:
                    invalid_controls.append(label)
                children[label] = child
            return True
        # Win32 explicitly leaves this function's return value unused. Validate
        # the enumerated controls instead of treating its BOOL as success.
        user.EnumChildWindows(hwnd, collect, 0)
        assert not invalid_controls, 'Unexpected native option control'
        assert len(children) == 5, 'Native option controls missing'
        assert user.SendMessageW(children[LABELS[initial]], 0xF0, 0, 0) == 1, 'Wrong default option'
        if choice is not None:
            user.SendMessageW(children[LABELS[choice]], 0xF5, 0, 0)
            assert user.SendMessageW(children[LABELS[choice]], 0xF0, 0, 0) == 1
        snapshot(hwnd, screenshot, user)
        # Post instead of synchronous Send: an MSI handler can enter its nested
        # install loop before returning, and the parent must retain its timeout.
        assert user.PostMessageW(children['取消' if cancel else '开始安装'], 0xF5, 0, 0)
        return finish()
    finally:
        if worker.is_alive() and hwnd:
            user.PostMessageW(hwnd, 0x10, 0, 0)
