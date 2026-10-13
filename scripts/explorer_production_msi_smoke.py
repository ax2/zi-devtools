"""Compile the unmodified shipping template and upgrade a pinned public MSI.

Installation is allowed only on disposable GitHub-hosted Windows runners.
The historical checksum proves download integrity, not publisher authentication.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib

from explorer_msi_actions_smoke import MENU_ROOTS, LEDGER_ROOT, RECOVERY_ROOT, read_tree, action_sequence, sanitize_log
from verify_release import verify
from installer_ui_probe import verify_tables, click

ROOT = Path(__file__).resolve().parents[1]


def main():
    if not __debug__:
        raise RuntimeError('Assertions required')
    parser = argparse.ArgumentParser()
    parser.add_argument('--compile-only', action='store_true')
    parser.add_argument('--baseline-dir', type=Path)
    parser.add_argument('--ui-probe', action='store_true')
    args = parser.parse_args()
    build = ROOT / 'target'
    observer = build / 'debug/examples/msi_fixture_state.exe'
    inspector = build / 'debug/examples/capability_inspect.exe'
    assert inspector.is_file(), 'Build the read-only capability inspector first'

    def observe(*values):
        return json.loads(subprocess.check_output([str(observer), *map(str, values)], timeout=30))

    if not args.compile_only:
        assert os.environ.get('GITHUB_ACTIONS') == 'true' and os.environ.get('CI') == 'true'
        assert os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted'
        assert observe('--products') == [], 'Existing product; refuse mutation'
        assert all(not read_tree(key) for key in MENU_ROOTS + [LEDGER_ROOT, RECOVERY_ROOT])
        assert args.baseline_dir is not None
        verify(args.baseline_dir, '0.81.0')
    base = build if args.compile_only else Path(os.environ['RUNNER_TEMP'])
    folder = Path(tempfile.mkdtemp(prefix='zi-msi-production-', dir=base)).resolve()
    assert folder.is_relative_to(base.resolve()) and not folder.is_symlink()
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text(encoding='utf-8'))['package']['version'].split('-')[0]
    payloads = {name: (build / 'release' / name).read_bytes() for name in ['ZiDevTools.exe', 'ZiDevToolsMcp.exe']}
    digest = hashlib.sha256(payloads['ZiDevTools.exe']).hexdigest()
    inspection = json.loads(subprocess.check_output([str(inspector), str(build / 'release/ZiDevTools.exe')], timeout=30))
    assert inspection['declarationPresent'] and inspection['declarationProperty']
    assert inspection['sha256'] == digest
    package = folder / 'current.msi'
    subprocess.run([os.environ.get('ZIDEVTOOLS_WIX', 'wix'), 'build', 'packaging/windows.wxs',
                    '-arch', 'x64', '-d', f'AppVersion={version}', '-d', f'AppExeSha256={digest}',
                    '-d', f"AppMenuCapabilities={inspection['declarationProperty']}",
                    '-d', f'MenuActionsDll={build / "release/zi_msi_actions.dll"}', '-o', str(package)],
                   cwd=ROOT, check=True, timeout=180)
    metadata = observe('--package', package)
    assert metadata['version'] == version and metadata['transactional_upgrade']
    order = action_sequence(package)
    required = ['InstallInitialize', 'ZiMenuPrepare', 'ZiMenuRollback', 'InstallFiles',
                'ZiMenuExecute', 'ZiMenuCommitReady', 'InstallExecute', 'RemoveExistingProducts',
                'InstallFinalize', 'ZiMenuCommitReceipt', 'ZiMenuFinalize']
    assert all(order[a] < order[b] for a, b in zip(required, required[1:]))
    proof = dict(productionTemplateUnmodified=True, appVersionUnchanged=True, msiFixtureVersion=version,
                 desktopSha256=digest, actionOrder={name: order[name] for name in required},
                 installationPerformed=False, legacyReleaseUpgradeVerified=False)
    proof['menuUiTables'] = verify_tables(package)
    (folder / 'proof.json').write_text(json.dumps(proof, indent=2) + '\n')
    if args.compile_only:
        print(f'PASS: unmodified production template compiled; no installation; evidence {folder}')
        return
    old = args.baseline_dir.resolve() / 'ZiDevTools-0.81.0-windows-x64.msi'
    previous = observe('--package', old)
    assert previous['version'] == '0.81.0'
    assert previous['desktop_component'] == metadata['desktop_component'], 'Historical component identity changed'
    target = folder / '安装 & spaces'
    api = ctypes.WinDLL('msi')
    for name, params in [('MsiInstallProductW', [ctypes.c_wchar_p, ctypes.c_wchar_p]),
                         ('MsiConfigureProductExW', [ctypes.c_wchar_p, ctypes.c_int, ctypes.c_int, ctypes.c_wchar_p]),
                         ('MsiEnableLogW', [ctypes.c_uint, ctypes.c_wchar_p, ctypes.c_uint]),
                         ('MsiSetInternalUI', [ctypes.c_uint, ctypes.c_void_p])]:
        getattr(api, name).argtypes = params
        getattr(api, name).restype = ctypes.c_uint
    api.MsiSetInternalUI(2, None)
    sequence = 0

    def invoke(label, msi, product, options='', state=None, expected=0):
        nonlocal sequence
        sequence += 1
        log = folder / f'{sequence:02}-{label}.log'
        assert api.MsiEnableLogW(0x3FFF, str(log), 2) == 0
        try:
            common = 'REBOOT=ReallySuppress ' + options
            if state is None:
                result = api.MsiInstallProductW(str(msi), common + f' INSTALLFOLDER="{target}"')
            else:
                result = api.MsiConfigureProductExW(product, 0, state, common)
        finally:
            api.MsiEnableLogW(0, None, 0)
        assert log.is_file(), f'Missing diagnostic log: {result}'
        text = sanitize_log(log)
        assert result == expected, f'{label} returned {result}, expected {expected}'
        assert observe().get('pending_msi') is None, 'Successful native installation retained pending menu state'
        return text

    code = metadata['product_code']
    old_code = previous['product_code']
    try:
        invoke('public-old', old, old_code)
        assert (target / 'ZiDevTools.exe').read_bytes() == (args.baseline_dir / 'ZiDevTools-0.81.0-windows-x64.exe').read_bytes()
        assert not observe()['entries'] and not any(read_tree(key) for key in MENU_ROOTS)
        marker = target / 'user-data-kept.txt'
        marker.write_bytes(b'not installer owned')
        invoke('public-upgrade-default-off', package, code)
        assert [item['product_code'] for item in observe('--products')] == [code]
        assert not observe()['entries'] and not any(read_tree(key) for key in MENU_ROOTS)
        assert all((target / name).read_bytes() == data for name, data in payloads.items())
        invoke('enable', package, code, 'ZIMENU=1 REINSTALL=ALL REINSTALLMODE=amus', 5)
        current = observe()
        assert len(current['entries']) == 1 and current['active'] is not None
        installation = current['entries'][0]['installation']
        assert installation['identity'] == {'kind': 'msi'}
        assert installation['version'] == version and installation['executable_sha256'] == digest
        expected_manifest = 'ZiDevTools/ShellCapabilities/1\nchoose-files\n' + ''.join(
            action + '\n' for action in inspection['actions'])
        assert installation['menu_capabilities'] == expected_manifest
        assert all(read_tree(key)['']['zicodebinarysha256'][0] == digest for key in MENU_ROOTS)
        invoke('inherited-repair', package, code, 'REINSTALL=ALL REINSTALLMODE=amus', 5)
        assert len(observe()['entries']) == 1 and all(read_tree(key) for key in MENU_ROOTS)
        invoke('disable', package, code, 'ZIMENU=0 REINSTALL=ALL REINSTALLMODE=amus', 5)
        assert not observe()['entries'] and not any(read_tree(key) for key in MENU_ROOTS)
        invoke('enable-again', package, code, 'ZIMENU=1 REINSTALL=ALL REINSTALLMODE=amus', 5)
        invoke('uninstall', package, code, state=2)
        assert not observe('--products') and not observe()['entries']
        assert not any(read_tree(key) for key in MENU_ROOTS)
        assert all(not (target / name).exists() for name in payloads)
        assert marker.read_bytes() == b'not installer owned'
        proof.update(installationPerformed=True, legacyReleaseUpgradeVerified=True,
                     baseline='public v0.81.0; checksum integrity only', defaultOff=True,
                     explicitEnableDisable=True, inheritedRepair=True, userDataPreserved=True,
                     productsAfter=[], menusAfter=False, desktopGuiVerified=False, explorerUiVerified=False)
        (folder / 'proof.json').write_text(json.dumps(proof, indent=2) + '\n')
        if args.ui_probe:
            keyboard_trace = []
            journal = observe('--journal')
            api.MsiSetInternalUI(5, None)
            click(lambda: invoke('ui-cancel', package, code, expected=1602), None, 'inherit', folder / 'ui-cancel.png', cancel=True, keyboard_trace=keyboard_trace)
            assert observe('--journal') == journal and not observe('--products')
            assert not observe()['entries'] and not any(read_tree(key) for key in MENU_ROOTS)
            assert all(not (target / name).exists() for name in payloads)
            click(lambda: invoke('ui-first-inherit', package, code), None, 'inherit', folder / 'ui-inherit-first.png', keyboard_trace=keyboard_trace)
            assert not observe()['entries'] and not any(read_tree(key) for key in MENU_ROOTS)
            repair = 'REINSTALL=ALL REINSTALLMODE=amus'
            click(lambda: invoke('ui-enable', package, code, repair + ' ZIMENU=0', 5), 'on', 'off', folder / 'ui-enable.png', keyboard_trace=keyboard_trace)
            assert len(observe()['entries']) == 1 and all(read_tree(key) for key in MENU_ROOTS)
            click(lambda: invoke('ui-inherit-enabled', package, code, repair, 5), None, 'inherit', folder / 'ui-inherit-enabled.png')
            assert len(observe()['entries']) == 1 and all(read_tree(key) for key in MENU_ROOTS)
            click(lambda: invoke('ui-disable', package, code, repair + ' ZIMENU=1', 5), 'off', 'on', folder / 'ui-disable.png', keyboard_trace=keyboard_trace)
            assert not observe()['entries'] and not any(read_tree(key) for key in MENU_ROOTS)
            click(lambda: invoke('ui-enable-before-uninstall', package, code, repair, 5), 'on', 'inherit', folder / 'ui-enable-before-uninstall.png')
            assert len(observe()['entries']) == 1 and all(read_tree(key) for key in MENU_ROOTS)
            fixture = folder / '真实文件 %1 & 空格.json'
            fixture.write_text('{"fixture":true}', encoding='utf-8')
            native_menu = json.loads(subprocess.check_output([
                str(build / 'debug/examples/explorer_installed_menu.exe'), str(fixture), str(target / 'ZiDevTools.exe')], timeout=30))
            assert native_menu['installedMsiMenuResolved'] and native_menu['nativeCommandCount'] == native_menu['receiverCount'] + 1
            assert native_menu['receiverCount'] >= 11 and not native_menu['commandInvocationPerformed']
            assert native_menu['desktopDeclarationMatches'] and not native_menu['mcpDeclaresDesktopReceivers']
            assert native_menu['declaredPlanMatchesShell']
            assert not native_menu['associationWrites'] and not native_menu['explorerWindowVerified']
            click(lambda: invoke('ui-full-uninstall', package, code, state=2), None, None, None, expect_options=False)
            assert not observe('--products') and marker.read_bytes() == b'not installer owned'
            assert not observe()['entries'] and not any(read_tree(key) for key in MENU_ROOTS)
            assert all(not (target / name).exists() for name in payloads)
            proof.update(nativeMenuUiClicks=True, cancelBeforeInstallation=True,
                         firstInheritOff=True, enabledRepairInherits=True,
                         commandLinePreselection=True, chosenToggleOverrides=True,
                         nativeUiScreenshots=6, fullUiUninstallSkipsMenuOptions=True,
                         fullUiUninstallRemovesEnabledMenu=True, installedShellMenu=native_menu,
                         keyboardNavigation=keyboard_trace, keyboardNavigationVerified=len(keyboard_trace) == 4,
                         keyboardAndScalingVerified=False, scalingVerified=False)
        (folder / 'proof.json').write_text(json.dumps(proof, indent=2) + '\n')
        print('PASS: public v0.81.0 upgrade through unmodified production template; repair/toggle/uninstall verified')
    finally:
        api.MsiSetInternalUI(2, None)
        # Only this isolated runner's two strictly inspected products may be removed.
        for installed in observe('--products'):
            assert installed['product_code'] in [code, old_code], 'Unexpected product; refuse cleanup'
            invoke('cleanup', package, installed['product_code'], state=2)


if __name__ == '__main__':
    main()
