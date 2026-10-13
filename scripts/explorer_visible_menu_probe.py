"""Inspect file, selected-directory, and directory-background MSI menus in Explorer.

This is intentionally limited to GitHub-hosted Windows runners. It creates
synthetic fixtures under RUNNER_TEMP, captures the real visible menus, and checks
the installed product cascade and receiver labels in all three contexts. It also
clicks the installed folder receiver and verifies its executable and exact folder
argument. The synthetic source file is never edited.
"""
import ctypes
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import time
import zlib


def require_hosted_runner():
    assert (os.environ.get('GITHUB_ACTIONS') == 'true'
            and os.environ.get('CI') == 'true'
            and os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted'), (
                'Visible Explorer probe requires a disposable hosted Windows runner')


def png_chunk(kind, data):
    return (struct.pack('>I', len(data)) + kind + data
            + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff))


def capture_desktop(path, user, gdi):
    """Capture the actual desktop, including the separate popup-menu HWND."""
    class BitmapInfoHeader(ctypes.Structure):
        _fields_ = [('size', ctypes.c_uint32), ('width', ctypes.c_int32),
                    ('height', ctypes.c_int32), ('planes', ctypes.c_uint16),
                    ('bit_count', ctypes.c_uint16), ('compression', ctypes.c_uint32),
                    ('image_size', ctypes.c_uint32), ('xppm', ctypes.c_int32),
                    ('yppm', ctypes.c_int32), ('used', ctypes.c_uint32),
                    ('important', ctypes.c_uint32)]

    class BitmapInfo(ctypes.Structure):
        _fields_ = [('header', BitmapInfoHeader), ('colors', ctypes.c_uint32 * 3)]

    user.GetSystemMetrics.argtypes = [ctypes.c_int]
    user.GetSystemMetrics.restype = ctypes.c_int
    user.GetDC.argtypes = [ctypes.c_void_p]
    user.GetDC.restype = ctypes.c_void_p
    user.ReleaseDC.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    user.ReleaseDC.restype = ctypes.c_int
    gdi.CreateCompatibleDC.argtypes = [ctypes.c_void_p]
    gdi.CreateCompatibleDC.restype = ctypes.c_void_p
    gdi.CreateCompatibleBitmap.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int]
    gdi.CreateCompatibleBitmap.restype = ctypes.c_void_p
    gdi.SelectObject.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    gdi.SelectObject.restype = ctypes.c_void_p
    gdi.BitBlt.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int,
                           ctypes.c_int, ctypes.c_int, ctypes.c_void_p,
                           ctypes.c_int, ctypes.c_int, ctypes.c_uint32]
    gdi.BitBlt.restype = ctypes.c_int
    gdi.GetDIBits.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint,
                              ctypes.c_uint, ctypes.c_void_p,
                              ctypes.POINTER(BitmapInfo), ctypes.c_uint]
    gdi.GetDIBits.restype = ctypes.c_int
    gdi.DeleteObject.argtypes = [ctypes.c_void_p]
    gdi.DeleteObject.restype = ctypes.c_int
    gdi.DeleteDC.argtypes = [ctypes.c_void_p]
    gdi.DeleteDC.restype = ctypes.c_int
    width, height = user.GetSystemMetrics(0), user.GetSystemMetrics(1)
    assert 0 < width <= 7680 and 0 < height <= 4320, (width, height)
    screen = user.GetDC(None)
    dc = gdi.CreateCompatibleDC(screen)
    bitmap = gdi.CreateCompatibleBitmap(screen, width, height)
    assert screen and dc and bitmap
    old = gdi.SelectObject(dc, bitmap)
    try:
        assert gdi.BitBlt(dc, 0, 0, width, height, screen, 0, 0, 0x00CC0020)
        gdi.SelectObject(dc, old)
        info = BitmapInfo()
        info.header = BitmapInfoHeader(
            ctypes.sizeof(BitmapInfoHeader), width, -height, 1, 32, 0,
            width * height * 4, 0, 0, 0, 0)
        pixels = (ctypes.c_ubyte * (width * height * 4))()
        assert gdi.GetDIBits(screen, bitmap, 0, height, pixels,
                             ctypes.byref(info), 0) == height
        bgra = bytes(pixels)
        rows = bytearray()
        for y in range(height):
            row = bgra[y * width * 4:(y + 1) * width * 4]
            rows.append(0)
            for x in range(0, len(row), 4):
                rows.extend((row[x + 2], row[x + 1], row[x], 255))
        header = struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)
        Path(path).write_bytes(b'\x89PNG\r\n\x1a\n'
                               + png_chunk(b'IHDR', header)
                               + png_chunk(b'IDAT', zlib.compress(bytes(rows), 6))
                               + png_chunk(b'IEND', b''))
    finally:
        gdi.DeleteObject(bitmap)
        gdi.DeleteDC(dc)
        user.ReleaseDC(None, screen)


def main():
    require_hosted_runner()
    assert len(sys.argv) == 3, 'Expected synthetic file and installed executable'
    source = Path(sys.argv[1]).resolve(strict=True)
    executable = Path(sys.argv[2]).resolve(strict=True)
    runner_temp = Path(os.environ['RUNNER_TEMP']).resolve(strict=True)
    assert source.is_file() and executable.is_file()
    assert source.is_relative_to(runner_temp) and executable.is_relative_to(runner_temp)
    assert source.name == '真实文件 %1 & 空格.json'
    original = b'synthetic installed Shell invocation fixture'
    assert source.read_bytes() == original
    output = source.parent

    user = ctypes.WinDLL('user32', use_last_error=True)
    gdi = ctypes.WinDLL('gdi32', use_last_error=True)
    callback = ctypes.WINFUNCTYPE(ctypes.c_bool, ctypes.c_void_p, ctypes.c_ssize_t)
    for name, args, result in [
        ('EnumWindows', [callback, ctypes.c_ssize_t], ctypes.c_bool),
        ('GetClassNameW', [ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_int], ctypes.c_int),
        ('GetWindowTextW', [ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_int], ctypes.c_int),
        ('IsWindowVisible', [ctypes.c_void_p], ctypes.c_bool),
        ('GetForegroundWindow', [], ctypes.c_void_p),
        ('ShowWindow', [ctypes.c_void_p, ctypes.c_int], ctypes.c_bool),
        ('GetWindowRect', [ctypes.c_void_p, ctypes.c_void_p], ctypes.c_bool),
        ('SetForegroundWindow', [ctypes.c_void_p], ctypes.c_bool),
        ('SendInput', [ctypes.c_uint, ctypes.c_void_p, ctypes.c_int], ctypes.c_uint),
        ('PostMessageW', [ctypes.c_void_p, ctypes.c_uint, ctypes.c_size_t,
                          ctypes.c_ssize_t], ctypes.c_bool),
        ('SetCursorPos', [ctypes.c_int, ctypes.c_int], ctypes.c_bool),
        ('GetMenuItemRect', [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint,
                             ctypes.c_void_p], ctypes.c_bool),
        ('GetWindowThreadProcessId', [ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint32)],
         ctypes.c_uint32),
        ('SendMessageW', [ctypes.c_void_p, ctypes.c_uint, ctypes.c_size_t,
                          ctypes.c_ssize_t], ctypes.c_ssize_t),
    ]:
        function = getattr(user, name)
        function.argtypes, function.restype = args, result
    for name, args, result in [
        ('GetMenuItemCount', [ctypes.c_void_p], ctypes.c_int),
        ('GetMenuItemID', [ctypes.c_void_p, ctypes.c_int], ctypes.c_uint),
        ('GetMenuStringW', [ctypes.c_void_p, ctypes.c_uint, ctypes.c_wchar_p,
                            ctypes.c_int, ctypes.c_uint], ctypes.c_int),
        ('GetSubMenu', [ctypes.c_void_p, ctypes.c_int], ctypes.c_void_p),
    ]:
        function = getattr(user, name)
        function.argtypes, function.restype = args, result

    class KeyInput(ctypes.Structure):
        _fields_ = [('virtual_key', ctypes.c_ushort), ('scan', ctypes.c_ushort),
                    ('flags', ctypes.c_uint32), ('time', ctypes.c_uint32),
                    ('extra', ctypes.c_void_p)]

    class MouseInput(ctypes.Structure):
        _fields_ = [('x', ctypes.c_int32), ('y', ctypes.c_int32),
                    ('data', ctypes.c_uint32), ('flags', ctypes.c_uint32),
                    ('time', ctypes.c_uint32), ('extra', ctypes.c_void_p)]

    class ScreenRect(ctypes.Structure):
        _fields_ = [('left', ctypes.c_long), ('top', ctypes.c_long),
                    ('right', ctypes.c_long), ('bottom', ctypes.c_long)]

    class InputUnion(ctypes.Union):
        _fields_ = [('key', KeyInput), ('mouse', MouseInput),
                    ('padding', ctypes.c_byte * 32)]

    class Input(ctypes.Structure):
        _fields_ = [('kind', ctypes.c_uint32), ('value', InputUnion)]

    def key(vk, down=True):
        item = Input(1, InputUnion(key=KeyInput(vk, 0, 0 if down else 2, 0, None)))
        assert user.SendInput(1, ctypes.byref(item), ctypes.sizeof(Input)) == 1

    def press(*keys):
        for value in keys:
            key(value)
        for value in reversed(keys):
            key(value, False)

    def click_at(x, y):
        assert user.SetCursorPos(x, y), f'Could not move to menu item at {x},{y}'
        for flag in (0x0002, 0x0004):  # left button down, then up
            item = Input(0, InputUnion(mouse=MouseInput(0, 0, 0, flag, 0, None)))
            assert user.SendInput(1, ctypes.byref(item), ctypes.sizeof(Input)) == 1

    def right_click_at(x, y):
        assert user.SetCursorPos(x, y), f'Could not move to blank folder area at {x},{y}'
        for flag in (0x0008, 0x0010):  # right button down, then up
            item = Input(0, InputUnion(mouse=MouseInput(0, 0, 0, flag, 0, None)))
            assert user.SendInput(1, ctypes.byref(item), ctypes.sizeof(Input)) == 1

    def receiver_process_info(hwnd):
        pid = ctypes.c_uint32()
        assert user.GetWindowThreadProcessId(hwnd, ctypes.byref(pid)) and pid.value
        command = (
            "$ErrorActionPreference='Stop'; "
            f"$p=Get-CimInstance Win32_Process -Filter 'ProcessId = {pid.value}'; "
            "if ($null -eq $p) { throw 'receiver process not found' }; "
            "$p | Select-Object ProcessId,ExecutablePath,CommandLine | ConvertTo-Json -Compress"
        )
        powershell = shutil.which('pwsh.exe') or shutil.which('powershell.exe')
        assert powershell, 'PowerShell is required to verify the launched receiver process'
        result = subprocess.run(
            [powershell, '-NoLogo', '-NoProfile', '-NonInteractive', '-Command', command],
            capture_output=True, text=True, encoding='utf-8', timeout=15, check=False)
        assert result.returncode == 0, f'Could not inspect receiver process {pid.value}'
        value = json.loads(result.stdout)
        assert int(value['ProcessId']) == pid.value
        return pid.value, value

    def verify_and_capture_receiver(menu, action_index, expected_folder, screenshot):
        baseline = {hwnd for hwnd, _, _ in top_windows()}
        action_rect = ScreenRect()
        assert user.GetMenuItemRect(explorer, menu, action_index, ctypes.byref(action_rect))
        assert action_rect.right > action_rect.left and action_rect.bottom > action_rect.top
        # Route the selected native Shell verb through Explorer's WM_COMMAND
        # owner. Hosted desktop sessions can swallow injected mouse-up and
        # keyboard input on nested context menus, even while visibly showing
        # the highlighted action.
        command_id = user.GetMenuItemID(menu, action_index)
        assert command_id != 0xFFFFFFFF, 'Folder receiver is not a native menu command'
        assert user.PostMessageW(explorer, 0x0111, command_id, 0), (
            'Could not dispatch the visible folder receiver through Explorer'
        )
        deadline = time.monotonic() + 30
        hwnd = None
        observed_windows = []
        while time.monotonic() < deadline:
            observed_windows = [
                (candidate, cls, title) for candidate, cls, title in top_windows()
                if candidate not in baseline
            ]
            matches = [
                (candidate, title) for candidate, _, title in observed_windows
                if title.startswith('Zi DevTools')
            ]
            if len(matches) == 1:
                hwnd = matches[0][0]
                break
            assert len(matches) <= 1, f'Multiple folder receiver windows appeared: {matches}'
            time.sleep(.2)
        assert hwnd, (
            'Clicking the folder receiver did not open a Zi DevTools window; '
            f'new visible windows: {observed_windows}'
        )
        pid, process = receiver_process_info(hwnd)
        image = Path(process['ExecutablePath']).resolve(strict=True)
        assert image == executable, 'Folder receiver did not launch the installed desktop EXE'
        command_line = process['CommandLine']
        shell = ctypes.WinDLL('shell32', use_last_error=True)
        shell.CommandLineToArgvW.argtypes = [ctypes.c_wchar_p, ctypes.POINTER(ctypes.c_int)]
        shell.CommandLineToArgvW.restype = ctypes.POINTER(ctypes.c_wchar_p)
        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        kernel.LocalFree.argtypes = [ctypes.c_void_p]
        kernel.LocalFree.restype = ctypes.c_void_p
        argc = ctypes.c_int()
        argv_pointer = shell.CommandLineToArgvW(command_line, ctypes.byref(argc))
        assert argv_pointer and argc.value > 0, 'Could not parse the launched receiver arguments'
        try:
            argv = [argv_pointer[index] for index in range(argc.value)]
        finally:
            kernel.LocalFree(argv_pointer)
        assert '--no-restore' in argv
        assert argv[argv.index('--tool') + 1] == 'disk-inspector'
        folder_argument = Path(argv[argv.index('--open-folder') + 1]).resolve(strict=True)
        assert folder_argument == expected_folder.resolve(strict=True), (
            f'Explorer supplied the wrong folder: {folder_argument}'
        )
        time.sleep(1.5)
        capture_desktop(screenshot, user, gdi)
        proof_result = dict(
            visibleWindow=True,
            installedExecutable=True,
            diskInspectorSelected=True,
            noServiceRestore=True,
            folderArgumentMatched=True,
            receiverProcessExited=False,
            windowScreenshot=str(screenshot),
        )

        # Close first. If the app hides to its tray, reap only this verified
        # runner-local process launched by the current test action.
        user.PostMessageW(hwnd, 0x0010, 0, 0)
        kernel.OpenProcess.argtypes = [ctypes.c_uint32, ctypes.c_bool, ctypes.c_uint32]
        kernel.OpenProcess.restype = ctypes.c_void_p
        kernel.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
        kernel.WaitForSingleObject.restype = ctypes.c_uint32
        kernel.TerminateProcess.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
        kernel.TerminateProcess.restype = ctypes.c_bool
        kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        kernel.CloseHandle.restype = ctypes.c_bool
        handle = kernel.OpenProcess(0x00100001, False, pid)  # SYNCHRONIZE | PROCESS_TERMINATE
        assert handle, 'Could not monitor the test-launched receiver process'
        try:
            state = kernel.WaitForSingleObject(handle, 8000)
            if state == 0x00000102:  # WAIT_TIMEOUT: close-to-tray kept the window process alive.
                assert image == executable and folder_argument == expected_folder.resolve(strict=True)
                assert kernel.TerminateProcess(handle, 0), 'Could not stop the verified runner receiver process'
                assert kernel.WaitForSingleObject(handle, 8000) == 0
            else:
                assert state == 0, f'Unexpected receiver process wait state: {state:#x}'
        finally:
            kernel.CloseHandle(handle)
        proof_result['receiverProcessExited'] = True
        return proof_result

    def text(hwnd, function):
        buffer = ctypes.create_unicode_buffer(1024)
        function(hwnd, buffer, len(buffer))
        return buffer.value

    def top_windows():
        values = []
        @callback
        def collect(hwnd, _):
            if user.IsWindowVisible(hwnd):
                values.append((hwnd, text(hwnd, user.GetClassNameW),
                               text(hwnd, user.GetWindowTextW)))
            return True
        assert user.EnumWindows(collect, 0)
        return values

    def menu_handle(hwnd):
        # MN_GETHMENU returns the HMENU owned by a native popup-menu window.
        return user.SendMessageW(hwnd, 0x01E1, 0, 0)

    def labels(menu):
        count = user.GetMenuItemCount(menu)
        assert 0 < count <= 128, f'Invalid visible menu item count: {count}'
        values = []
        for index in range(count):
            buffer = ctypes.create_unicode_buffer(512)
            length = user.GetMenuStringW(menu, index, buffer, len(buffer), 0x400)
            assert 0 <= length < len(buffer), f'Cannot read menu item {index}'
            values.append(buffer.value if length else '')  # Native separators have no label.
        return values

    fixture_dir = source.parent.name
    selected_folder = output / 'A-文件夹菜单测试'
    explorer = None
    root_menu_window = None
    proof = {'fixtureUnchanged': False, 'explorerWindowVisible': False,
             'fileSelectionInputSent': False,
             'contextMenuVisible': False, 'productCascadeVisible': False,
             'productCascadeClicked': False, 'genericReceiverVisible': False,
             'directoryMenuVisible': False, 'directoryCascadeVisible': False,
             'directoryReceiverLaunched': False,
             'backgroundMenuVisible': False, 'backgroundCascadeVisible': False,
             'backgroundReceiverLaunched': False}
    try:
        existing_windows = {hwnd for hwnd, cls, _ in top_windows() if cls == 'CabinetWClass'}
        os.startfile(str(source.parent))
        deadline = time.monotonic() + 30
        last_windows = []
        while time.monotonic() < deadline:
            last_windows = [(hwnd, cls, title) for hwnd, cls, title in top_windows()
                            if cls == 'CabinetWClass']
            matches = [(hwnd, cls, title) for hwnd, cls, title in last_windows
                       if fixture_dir.lower() in title.lower()]
            if not matches:
                # Explorer may omit the folder name from its modern window title.
                matches = [(hwnd, cls, title) for hwnd, cls, title in last_windows
                           if hwnd not in existing_windows]
            if len(matches) == 1:
                explorer = matches[0][0]
                break
            assert len(matches) <= 1, f'Multiple new Explorer windows: {matches}'
            time.sleep(.2)
        assert explorer, (f'Explorer did not open the disposable folder {fixture_dir}; '
                          f'visible File Explorer windows were {last_windows}')
        proof['explorerWindowVisible'] = True
        assert user.SetForegroundWindow(explorer)
        user.ShowWindow(explorer, 3)  # maximize so background clicks land in the file view
        time.sleep(1)
        # The fixture folder contains exactly one file, so this focuses/selects
        # only the synthetic file even if Explorer did not honor /select.
        press(0x24)  # Home: move to the only item in the file view.
        press(0x11, 0x41)  # Ctrl+A: select the only item, never multiple files.
        proof['fileSelectionInputSent'] = True
        time.sleep(.2)
        press(0xA0, 0x79)  # Shift+F10 opens the selected file's classic context menu.
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            menus = [hwnd for hwnd, cls, _ in top_windows() if cls == '#32768']
            if menus:
                root_menu_window = menus[-1]
                break
            time.sleep(.1)
        assert root_menu_window, 'Explorer did not display a native context-menu popup'
        popup = menu_handle(root_menu_window)
        root_labels = labels(popup)
        proof['contextMenuVisible'] = True
        proof['menuVisibleBeforeRefresh'] = root_labels.count('Zi DevTools') == 1
        proof['rootMenuLabelsBeforeRefresh'] = root_labels
        proof['rootScreenshot'] = str(output / 'explorer-menu-root.png')
        capture_desktop(proof['rootScreenshot'], user, gdi)
        if not proof['menuVisibleBeforeRefresh']:
            # Diagnostic only: a per-user association refresh should already
            # have happened inside the MSI commit path. Record whether a same-
            # user refresh changes what this Explorer window displays.
            press(0x1B)
            shell = ctypes.WinDLL('shell32', use_last_error=True)
            shell.SHChangeNotify.argtypes = [ctypes.c_int, ctypes.c_uint,
                                             ctypes.c_void_p, ctypes.c_void_p]
            shell.SHChangeNotify.restype = None
            shell.SHChangeNotify(0x08000000, 0x3000, None, None)
            time.sleep(.5)
            press(0xA0, 0x79)
            deadline = time.monotonic() + 10
            root_menu_window = None
            while time.monotonic() < deadline:
                menus = [hwnd for hwnd, cls, _ in top_windows() if cls == '#32768']
                if menus:
                    root_menu_window = menus[-1]
                    break
                time.sleep(.1)
            assert root_menu_window, 'Explorer did not reopen its context menu after association refresh'
            popup = menu_handle(root_menu_window)
            root_labels = labels(popup)
            proof['menuVisibleAfterRefresh'] = root_labels.count('Zi DevTools') == 1
            proof['rootMenuLabelsAfterRefresh'] = root_labels
            proof['refreshScreenshot'] = str(output / 'explorer-menu-after-refresh.png')
            capture_desktop(proof['refreshScreenshot'], user, gdi)
            assert proof['menuVisibleBeforeRefresh'], (
                'MSI menu was absent until an external SHChangeNotify; '
                f'visibleAfterRefresh={proof["menuVisibleAfterRefresh"]}; '
                f'before={proof["rootMenuLabelsBeforeRefresh"]}; after={root_labels}')
        assert root_labels.count('Zi DevTools') == 1, root_labels
        proof['rootMenuLabels'] = root_labels

        item_index = root_labels.index('Zi DevTools')
        rect = ScreenRect()
        assert user.GetMenuItemRect(explorer, popup, item_index, ctypes.byref(rect))
        assert rect.right > rect.left and rect.bottom > rect.top, rect
        click_at((rect.left + rect.right) // 2, (rect.top + rect.bottom) // 2)
        time.sleep(.35)
        submenu = user.GetSubMenu(popup, item_index)
        if not submenu:
            # The Shell popup may replace its HMENU while entering the cascade.
            popup = menu_handle(root_menu_window)
            submenu = user.GetSubMenu(popup, item_index)
        assert submenu, 'Visible Zi DevTools menu item did not open a submenu'
        child_labels = labels(submenu)
        assert child_labels.count('选择工具处理…') == 1, child_labels
        proof['productCascadeVisible'] = True
        proof['productCascadeClicked'] = True
        proof['genericReceiverVisible'] = True
        proof['receiverMenuLabels'] = child_labels
        proof['receiverScreenshot'] = str(output / 'explorer-menu-receivers.png')
        capture_desktop(proof['receiverScreenshot'], user, gdi)
        proof['fixtureUnchanged'] = source.read_bytes() == original
        assert proof['fixtureUnchanged'], 'Explorer menu interaction changed the synthetic file'

        # Add one empty child after the one-file receiver check. Explorer sorts
        # folders first, so Home selects this directory without selecting files.
        selected_folder.mkdir()
        press(0x1B)
        press(0x1B)
        press(0x74)  # F5 refresh the disposable folder view.
        time.sleep(.5)
        press(0x24)  # Home selects the first (folder) item.
        press(0xA0, 0x79)  # Shift+F10 opens the selected folder menu.
        deadline = time.monotonic() + 10
        root_menu_window = None
        while time.monotonic() < deadline:
            menus = [hwnd for hwnd, cls, _ in top_windows() if cls == '#32768']
            if menus:
                root_menu_window = menus[-1]
                break
            time.sleep(.1)
        assert root_menu_window, 'Explorer did not display the selected-directory context menu'
        popup = menu_handle(root_menu_window)
        directory_labels = labels(popup)
        assert directory_labels.count('Zi DevTools') == 1, directory_labels
        proof['directoryMenuVisible'] = True
        proof['directoryRootLabels'] = directory_labels
        proof['directoryScreenshot'] = str(output / 'explorer-menu-directory.png')
        capture_desktop(proof['directoryScreenshot'], user, gdi)
        directory_index = directory_labels.index('Zi DevTools')
        rect = ScreenRect()
        assert user.GetMenuItemRect(explorer, popup, directory_index, ctypes.byref(rect))
        click_at((rect.left + rect.right) // 2, (rect.top + rect.bottom) // 2)
        time.sleep(.35)
        submenu = user.GetSubMenu(popup, directory_index)
        if not submenu:
            popup = menu_handle(root_menu_window)
            submenu = user.GetSubMenu(popup, directory_index)
        assert submenu, 'Directory Zi DevTools item did not open a submenu'
        directory_actions = labels(submenu)
        assert directory_actions.count('分析目录空间…') == 1, directory_actions
        proof['directoryCascadeVisible'] = True
        proof['directoryReceiverLabels'] = directory_actions
        proof['directoryReceiverScreenshot'] = str(output / 'explorer-menu-directory-action.png')
        capture_desktop(proof['directoryReceiverScreenshot'], user, gdi)
        proof['directoryReceiverInvocation'] = verify_and_capture_receiver(
            submenu, directory_actions.index('分析目录空间…'), selected_folder,
            output / 'explorer-directory-analysis-launched.png')
        proof['directoryReceiverLaunched'] = True

        # Exercise the true Directory\Background object by right-clicking blank
        # space and verify its %V expansion reaches the installed receiver.
        press(0x1B)
        press(0x1B)
        rect = ScreenRect()
        assert user.GetWindowRect(explorer, ctypes.byref(rect))
        assert rect.right - rect.left > 500 and rect.bottom - rect.top > 400, rect
        right_click_at(rect.right - 100, rect.bottom - 110)
        deadline = time.monotonic() + 10
        root_menu_window = None
        while time.monotonic() < deadline:
            menus = [hwnd for hwnd, cls, _ in top_windows() if cls == '#32768']
            if menus:
                root_menu_window = menus[-1]
                break
            time.sleep(.1)
        assert root_menu_window, 'Explorer did not display the folder-background context menu'
        popup = menu_handle(root_menu_window)
        background_labels = labels(popup)
        assert background_labels.count('Zi DevTools') == 1, background_labels
        proof['backgroundMenuVisible'] = True
        proof['backgroundRootLabels'] = background_labels
        proof['backgroundScreenshot'] = str(output / 'explorer-menu-background.png')
        capture_desktop(proof['backgroundScreenshot'], user, gdi)
        background_index = background_labels.index('Zi DevTools')
        rect = ScreenRect()
        assert user.GetMenuItemRect(explorer, popup, background_index, ctypes.byref(rect))
        click_at((rect.left + rect.right) // 2, (rect.top + rect.bottom) // 2)
        time.sleep(.35)
        submenu = user.GetSubMenu(popup, background_index)
        if not submenu:
            popup = menu_handle(root_menu_window)
            submenu = user.GetSubMenu(popup, background_index)
        assert submenu, 'Background Zi DevTools item did not open a submenu'
        background_actions = labels(submenu)
        assert background_actions.count('分析目录空间…') == 1, background_actions
        proof['backgroundCascadeVisible'] = True
        proof['backgroundReceiverLabels'] = background_actions
        proof['backgroundReceiverScreenshot'] = str(output / 'explorer-menu-background-action.png')
        capture_desktop(proof['backgroundReceiverScreenshot'], user, gdi)
        proof['backgroundReceiverInvocation'] = verify_and_capture_receiver(
            submenu, background_actions.index('分析目录空间…'), output,
            output / 'explorer-background-analysis-launched.png')
        proof['backgroundReceiverLaunched'] = True
        proof['fixtureUnchanged'] = source.read_bytes() == original
        assert proof['fixtureUnchanged'], 'Explorer menu interaction changed the synthetic file'
        (output / 'explorer-visible-proof.json').write_text(
            json.dumps(proof, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
        print(json.dumps(proof, ensure_ascii=True))
    finally:
        # Dismiss our menus and close only the uniquely named disposable window.
        try:
            press(0x1B)
            press(0x1B)
        except (AssertionError, OSError):
            pass
        if explorer:
            user.PostMessageW(explorer, 0x0010, 0, 0)
        if selected_folder.exists():
            assert selected_folder.parent == output
            shutil.rmtree(selected_folder)
        proof['fixtureUnchanged'] = source.read_bytes() == original


if __name__ == '__main__':
    main()
