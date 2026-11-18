"""Bounded native popup snapshots; no input injection or window creation."""
import ctypes
import time


class MenuUnavailable(RuntimeError):
    """The popup is not ready or changed while its snapshot was read."""


def menu_labels(user, menu):
    ctypes.set_last_error(0)
    count = user.GetMenuItemCount(menu)
    error = ctypes.get_last_error()
    if count <= 0:
        raise MenuUnavailable(f'GetMenuItemCount={count}, winerror={error}, hmenu={menu!r}')
    assert count <= 128, f'Unexpected visible menu item count: {count}'
    values = []
    for index in range(count):
        buffer = ctypes.create_unicode_buffer(512)
        length = user.GetMenuStringW(menu, index, buffer, len(buffer), 0x400)
        assert 0 <= length < len(buffer) - 1, f'Cannot read complete menu item {index}'
        values.append(buffer.value if length else '')  # Separators have no label.
    if not user.IsMenu(menu) or user.GetMenuItemCount(menu) != count:
        raise MenuUnavailable(f'Menu changed during label read, hmenu={menu!r}')
    return values


def popup_windows_for_process(user, windows, process_id):
    owned = []
    for hwnd, class_name, _ in windows:
        if class_name == '#32768':
            owner = ctypes.c_uint32()
            user.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
            if owner.value == process_id:
                owned.append(hwnd)
    return owned


def wait_menu_snapshot(user, windows, menu_handle, description, *, timeout=10,
                       diagnostics=None, clock=time.monotonic, sleep=time.sleep):
    """Reacquire HWND + HMENU together; never retry missing product labels.

    The caller supplies only popups owned by its disposable Explorer process.
    A visible HWND alone does not prove its menu is already usable. Do not keep
    a stale HWND/HMENU across polls or turn an absent product menu into success.
    """
    diagnostic = diagnostics if diagnostics is not None else {}
    diagnostic.update(attempts=0, transientReads=0, lastUnavailable='No owned popup')
    deadline = clock() + timeout
    while clock() < deadline:
        diagnostic['attempts'] += 1
        for hwnd in windows():
            menu = menu_handle(hwnd)
            try:
                values = menu_labels(user, menu)
                if not user.IsWindowVisible(hwnd) or menu_handle(hwnd) != menu:
                    raise MenuUnavailable(f'Popup changed during label read, hwnd={hwnd!r}')
            except MenuUnavailable as error:
                diagnostic['transientReads'] += 1
                diagnostic['lastUnavailable'] = str(error)
                continue
            return hwnd, menu, values
        sleep(min(.05, max(0, deadline - clock())))
    raise AssertionError(f'Timed out reading {description}: {diagnostic}')
