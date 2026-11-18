"""Run with unittest on Windows. Creates only invisible, process-owned HMENUs.

Never opens Explorer, sends input, captures a screen or bypasses the hosted guard.
"""
import ctypes
import os
from pathlib import Path
import unittest
from unittest.mock import patch

from explorer_menu_snapshot import (MenuUnavailable, menu_labels,
                                    popup_windows_for_process, wait_menu_snapshot)


class Clock:
    def __init__(self):
        self.now = 0

    def __call__(self):
        return self.now

    def sleep(self, duration):
        self.now += duration


@unittest.skipUnless(os.name == 'nt', 'requires real Win32 menu objects')
class MenuSnapshotTests(unittest.TestCase):
    def setUp(self):
        self.user = ctypes.WinDLL('user32', use_last_error=True)
        for name, args, result in [
            ('CreatePopupMenu', [], ctypes.c_void_p),
            ('AppendMenuW', [ctypes.c_void_p, ctypes.c_uint, ctypes.c_size_t,
                             ctypes.c_wchar_p], ctypes.c_bool),
            ('DestroyMenu', [ctypes.c_void_p], ctypes.c_bool),
            ('IsMenu', [ctypes.c_void_p], ctypes.c_bool),
            ('GetMenuItemCount', [ctypes.c_void_p], ctypes.c_int),
            ('GetMenuStringW', [ctypes.c_void_p, ctypes.c_uint, ctypes.c_wchar_p,
                                ctypes.c_int, ctypes.c_uint], ctypes.c_int),
        ]:
            function = getattr(self.user, name)
            function.argtypes, function.restype = args, result
        self.menus = []
        self.clock = Clock()

    def tearDown(self):
        for menu in self.menus:
            if self.user.IsMenu(menu):
                self.assertTrue(self.user.DestroyMenu(menu))

    def menu(self, *labels):
        menu = self.user.CreatePopupMenu()
        self.assertTrue(menu)
        self.menus.append(menu)
        for index, label in enumerate(labels):
            flags = 0x800 if label is None else 0  # MF_SEPARATOR / MF_STRING
            self.assertTrue(self.user.AppendMenuW(menu, flags, index + 1, label))
        return menu

    def wait(self, handles, user=None, windows=lambda: [123]):
        values = iter(handles)
        last = handles[-1]
        diagnostic = {}
        result = wait_menu_snapshot(user or self.wrapper(), windows,
                                    lambda _: next(values, last), 'folder-background',
                                    timeout=.15, diagnostics=diagnostic,
                                    clock=self.clock, sleep=self.clock.sleep)
        return result, diagnostic

    def wrapper(self, **overrides):
        # No HWND is created: visibility/ownership are separately tested inputs.
        class User:
            IsWindowVisible = staticmethod(lambda _: True)
        user = User()
        for name in ('GetMenuItemCount', 'GetMenuStringW', 'IsMenu'):
            setattr(user, name, overrides.get(name, getattr(self.user, name)))
        return user

    def test_actual_win32_unicode_separator_snapshot(self):
        menu = self.menu('Zi DevTools', None, '分析目录空间…')
        self.assertEqual(menu_labels(self.user, menu), ['Zi DevTools', '', '分析目录空间…'])

    def test_actual_destroyed_hmenu_reproduces_minus_one(self):
        menu = self.menu('Zi DevTools')
        self.assertTrue(self.user.DestroyMenu(menu))
        self.assertEqual(self.user.GetMenuItemCount(menu), -1)
        with self.assertRaisesRegex(MenuUnavailable, 'GetMenuItemCount=-1, winerror=1401'):
            menu_labels(self.user, menu)

    def test_reacquires_after_actual_destroyed_menu(self):
        stale = self.menu('old menu')
        live = self.menu('Zi DevTools')  # Allocate before destruction: no handle reuse.
        self.assertTrue(self.user.DestroyMenu(stale))
        (hwnd, menu, labels), diagnostic = self.wait([stale, live, live])
        self.assertEqual((hwnd, menu, labels), (123, live, ['Zi DevTools']))
        self.assertEqual(diagnostic['transientReads'], 1)
        self.assertEqual(diagnostic['attempts'], 2)
        self.assertIn('winerror=1401', diagnostic['lastUnavailable'])

    def test_permanent_destroyed_menu_fails_with_bounded_diagnostic(self):
        menu = self.menu('old')
        self.assertTrue(self.user.DestroyMenu(menu))
        with self.assertRaisesRegex(AssertionError, 'Timed out reading folder-background.*GetMenuItemCount=-1'):
            self.wait([menu])
        self.assertAlmostEqual(self.clock.now, .15)

    def test_no_owned_popup_fails_without_reading_other_windows(self):
        with self.assertRaisesRegex(AssertionError, 'No owned popup'):
            self.wait([0], windows=lambda: [])
        self.assertAlmostEqual(self.clock.now, .15)

    def test_empty_menu_can_initialize(self):
        empty, live = self.menu(), self.menu('Zi DevTools')
        (_, _, labels), diagnostic = self.wait([empty, live, live])
        self.assertEqual(labels, ['Zi DevTools'])
        self.assertEqual(diagnostic['transientReads'], 1)

    def test_valid_menu_missing_product_is_not_retried_or_fabricated(self):
        menu = self.menu('Unrelated item')
        (_, _, labels), diagnostic = self.wait([menu])
        self.assertEqual(labels.count('Zi DevTools'), 0)
        self.assertEqual(diagnostic['attempts'], 1)
        self.assertEqual(diagnostic['transientReads'], 0)
        with self.assertRaises(AssertionError):
            assert labels.count('Zi DevTools') == 1  # Same product assertion as caller.

    def test_replacement_after_read_discards_old_labels(self):
        old, new = self.menu('old'), self.menu('Zi DevTools')
        (_, handle, labels), diagnostic = self.wait([old, new, new, new])
        self.assertEqual((handle, labels), (new, ['Zi DevTools']))
        self.assertEqual(diagnostic['transientReads'], 1)

    def test_change_during_label_read_discards_partial_snapshot(self):
        old, new = self.menu('old'), self.menu('Zi DevTools')
        destroyed = False

        def read(menu, *args):
            nonlocal destroyed
            result = self.user.GetMenuStringW(menu, *args)
            if menu == old and not destroyed:
                self.assertTrue(self.user.DestroyMenu(old))
                destroyed = True
            return result

        (_, handle, labels), diagnostic = self.wait([old, new, new],
                                                   user=self.wrapper(GetMenuStringW=read))
        self.assertEqual((handle, labels), (new, ['Zi DevTools']))
        self.assertEqual(diagnostic['transientReads'], 1)

    def test_oversized_and_truncated_menus_are_hard_failures(self):
        with self.assertRaisesRegex(AssertionError, 'Unexpected visible menu item count: 129'):
            self.wait([self.menu(*(['item'] * 129))])
        with self.assertRaisesRegex(AssertionError, 'Cannot read complete menu item'):
            self.wait([self.menu('x' * 512)])

    def test_foreign_process_popup_is_excluded(self):
        class User:
            @staticmethod
            def GetWindowThreadProcessId(hwnd, pointer):
                ctypes.cast(pointer, ctypes.POINTER(ctypes.c_uint32))[0] = {1: 41, 2: 42}[hwnd]
                return 1
        windows = [(1, '#32768', ''), (2, '#32768', ''), (3, 'other', '')]
        self.assertEqual(popup_windows_for_process(User(), windows, 41), [1])

    def test_visible_explorer_probe_still_refuses_developer_desktop(self):
        from explorer_visible_menu_probe import require_hosted_runner
        with patch.dict(os.environ, {'GITHUB_ACTIONS': '', 'CI': '', 'RUNNER_ENVIRONMENT': ''}):
            with self.assertRaisesRegex(AssertionError, 'disposable hosted Windows runner'):
                require_hosted_runner()


if __name__ == '__main__':
    unittest.main()
