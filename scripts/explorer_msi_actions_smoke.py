"""Candidate DLL inside real MSI transactions on a disposable Windows runner.

Production WXS is unchanged. The candidate finalizes its journal automatically
after InstallFinalize; the observer reads evidence and never completes success.
"""
import argparse
import ctypes
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess
import tempfile
import uuid
import winreg
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
NS = '{http://wixtoolset.org/schemas/v4/wxs}'
ET.register_namespace('', NS[1:-1])
MENU_ROOTS = [r'Software\Classes\*\shell\ZiCode.DevTools', r'Software\Classes\ZiCode.DevTools.FileActions']
LEDGER_ROOT = r'Software\ZiCode\ZiDevTools\ExplorerInstallations'
RECOVERY_ROOT = r'Software\ZiCode\ZiDevTools\ExplorerRecovery'


def action_sequence(path):
    api = ctypes.WinDLL('msi')
    for name, params, result in [
        ('MsiOpenDatabaseW', [ctypes.c_wchar_p, ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint)], ctypes.c_uint),
        ('MsiDatabaseOpenViewW', [ctypes.c_uint, ctypes.c_wchar_p, ctypes.POINTER(ctypes.c_uint)], ctypes.c_uint),
        ('MsiViewExecute', [ctypes.c_uint, ctypes.c_uint], ctypes.c_uint),
        ('MsiViewFetch', [ctypes.c_uint, ctypes.POINTER(ctypes.c_uint)], ctypes.c_uint),
        ('MsiRecordGetStringW', [ctypes.c_uint, ctypes.c_uint, ctypes.c_wchar_p, ctypes.POINTER(ctypes.c_uint)], ctypes.c_uint),
        ('MsiRecordGetInteger', [ctypes.c_uint, ctypes.c_uint], ctypes.c_int),
        ('MsiCloseHandle', [ctypes.c_uint], ctypes.c_uint),
    ]:
        function = getattr(api, name)
        function.argtypes, function.restype = params, result
    database, view = ctypes.c_uint(), ctypes.c_uint()
    assert api.MsiOpenDatabaseW(str(path), None, ctypes.byref(database)) == 0
    try:
        assert api.MsiDatabaseOpenViewW(database, 'SELECT `Action`, `Sequence` FROM `InstallExecuteSequence`', ctypes.byref(view)) == 0
        assert api.MsiViewExecute(view, 0) == 0
        actions = {}
        while True:
            record = ctypes.c_uint()
            code = api.MsiViewFetch(view, ctypes.byref(record))
            if code == 259:
                break
            assert code == 0 and len(actions) < 128
            try:
                name, size = ctypes.create_unicode_buffer(256), ctypes.c_uint(256)
                assert api.MsiRecordGetStringW(record, 1, name, ctypes.byref(size)) == 0
                actions[name.value] = api.MsiRecordGetInteger(record, 2)
            finally:
                api.MsiCloseHandle(record)
        return actions
    finally:
        if view.value:
            api.MsiCloseHandle(view)
        api.MsiCloseHandle(database)


def read_tree(path):
    result = {}
    def visit(key, relative):
        assert len(result) < 256
        values = {}
        index = 0
        while True:
            try:
                name, value, kind = winreg.EnumValue(key, index)
            except OSError as error:
                if error.winerror != 259:
                    raise
                break
            assert index < 1024
            values[name.lower()] = (value, kind)
            index += 1
        result[relative.lower()] = values
        index = 0
        while True:
            try:
                name = winreg.EnumKey(key, index)
            except OSError as error:
                if error.winerror != 259:
                    raise
                break
            with winreg.OpenKey(key, name) as child:
                visit(child, relative + '\\' + name)
            index += 1
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, path) as key:
            visit(key, '')
    except FileNotFoundError:
        pass
    return result


def main():
    if not __debug__:
        raise RuntimeError('Assertions required')
    parser = argparse.ArgumentParser()
    parser.add_argument('--compile-only', action='store_true')
    args = parser.parse_args()
    build = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target'))
    probe = build / 'debug/examples/msi_fixture_state.exe'
    actions = build / 'debug/zi_msi_actions.dll'
    assert probe.is_file() and actions.is_file(), 'Build candidate DLL and core observer first'
    def observe(*arguments):
        return json.loads(subprocess.check_output([str(probe), *map(str, arguments)], timeout=30))
    if not args.compile_only:
        assert os.environ.get('GITHUB_ACTIONS') == 'true' and os.environ.get('CI') == 'true'
        assert os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted'
        assert observe('--products') == [], 'Existing product; refuse mutation'
        for path in MENU_ROOTS + [LEDGER_ROOT, RECOVERY_ROOT]:
            assert not read_tree(path), 'Existing registration; refuse mutation'
    base = build if args.compile_only else Path(os.environ['RUNNER_TEMP'])
    folder = Path(tempfile.mkdtemp(prefix='zi-msi-menu-', dir=base)).resolve()
    assert folder.is_relative_to(base.resolve()) and not folder.is_symlink()
    target = folder / '安装 & spaces'
    packages = {}
    compile_orders = {}
    for label, version, fault in [('old', '0.1.0', None), ('before', '0.2.0', 'before'),
                                   ('after', '0.2.0', 'after'), ('new', '0.2.0', None)]:
        payload = folder / label
        payload.mkdir()
        # Synthetic MZ payloads satisfy the product's documented basic check;
        # never execute them or claim full PE/Shell/UI acceptance.
        content = b'MZ' + (b'old' if label == 'old' else b'new') * 64
        for name in ['ZiDevTools.exe', 'ZiDevToolsMcp.exe']:
            (payload / name).write_bytes(content)
        tree = ET.parse(ROOT / 'packaging/windows.wxs')
        package = tree.getroot().find(NS + 'Package')
        package.set('Version', version)
        # Rollback CA itself generates script, so REP cannot follow it in the
        # afterInstallInitialize slot (MSI error 2613). Install replacement files
        # first with stable component GUIDs, then remove old clients inside the
        # same transaction. File rollback precedes our early rollback callback.
        package.find(NS + 'MajorUpgrade').set('Schedule', 'afterInstallExecute')
        for file in package.iter(NS + 'File'):
            file.set('Source', str(payload / Path(file.get('Source')).name)
                     if file.get('Id') in ['ZiDevToolsExe', 'ZiDevToolsMcpExe'] else str(ROOT / 'LICENSE'))
        ET.SubElement(package, NS + 'Property', Id='ZIMENU', Secure='yes')
        ET.SubElement(package, NS + 'Property', Id='ZiMenuBinarySha256', Value=hashlib.sha256(content).hexdigest())
        ET.SubElement(package, NS + 'Binary', Id='ZiMenuActions', SourceFile=str(actions))
        for name, execution in [('Prepare', 'immediate'), ('Rollback', 'rollback'),
                                ('Execute', 'deferred'), ('Finalize', 'immediate')]:
            ET.SubElement(package, NS + 'CustomAction', Id='ZiMenu' + name,
                          BinaryRef='ZiMenuActions', DllEntry='ZiMenu' + name,
                          Execute=execution, Return='ignore' if name == 'Finalize' else 'check', Impersonate='yes')
        sequence = ET.SubElement(package, NS + 'InstallExecuteSequence')
        # Queue menu rollback before file writes so reverse execution restores
        # files before verifying the old executable and restoring its menu.
        ET.SubElement(sequence, NS + 'Custom', Action='ZiMenuPrepare', Before='ZiMenuRollback', Condition='1')
        ET.SubElement(sequence, NS + 'Custom', Action='ZiMenuRollback', After='InstallInitialize', Condition='ZiMenuRollback')
        ET.SubElement(sequence, NS + 'Custom', Action='ZiMenuExecute', After='InstallFiles', Condition='ZiMenuExecute')
        # InstallFinalize ends the MSI transaction. Failure here must leave the
        # Applied journal recoverable, never falsely request rollback of files
        # that have already been committed. No observer-assisted success path.
        ET.SubElement(sequence, NS + 'Custom', Action='ZiMenuFinalize', After='InstallFinalize', Condition='ZiMenuExecute')
        # Explicitly flush the deferred script before the late Type-19 failure;
        # without this action a pre-Finalize error would precede file/menu writes.
        ET.SubElement(sequence, NS + 'InstallExecute', Sequence='6500')
        if fault:
            ET.SubElement(package, NS + 'CustomAction', Id='MenuFixtureFailure', Error='Intentional menu rollback fixture')
            position = {'Sequence': '1520'} if fault == 'before' else {'Before': 'InstallFinalize'}
            ET.SubElement(sequence, NS + 'Custom', Action='MenuFixtureFailure', Condition='NOT REMOVE', **position)
        source = folder / (label + '.wxs')
        tree.write(source, encoding='utf-8', xml_declaration=True)
        msi = folder / (label + '.msi')
        subprocess.run([os.environ.get('ZIDEVTOOLS_WIX', 'wix'), 'build', str(source), '-arch', 'x64', '-o', str(msi)],
                       check=True, cwd=ROOT, timeout=180)
        metadata = observe('--package', msi)
        assert metadata['version'] == version and metadata['transactional_upgrade']
        order = action_sequence(msi)
        required = ['InstallInitialize', 'ZiMenuPrepare', 'ZiMenuRollback',
                    'InstallFiles', 'ZiMenuExecute', 'InstallExecute', 'RemoveExistingProducts',
                    'InstallFinalize', 'ZiMenuFinalize']
        positions = [order[name] for name in required]
        assert all(a < b for a, b in zip(positions, positions[1:])), dict(zip(required, positions))
        if fault == 'before':
            assert order['ZiMenuRollback'] < order['MenuFixtureFailure'] < order['InstallFiles']
        if fault == 'after':
            assert order['ZiMenuExecute'] < order['InstallExecute'] < order['RemoveExistingProducts'] < order['MenuFixtureFailure'] < order['InstallFinalize']
        compile_orders[label] = dict(zip(required, positions))
        packages[label] = (msi, metadata, content)
    assert len({value[1]['desktop_component'] for value in packages.values()}) == 1
    (folder / 'compile-proof.json').write_text(json.dumps(dict(sequences=compile_orders,
        dllSha256=hashlib.sha256(actions.read_bytes()).hexdigest()), indent=2) + '\n')
    if args.compile_only:
        print('PASS: four candidate DLL fixtures compiled, no installation performed')
        return

    native = ctypes.WinDLL(str(actions))
    for name in ['ZiMenuPrepare', 'ZiMenuExecute', 'ZiMenuRollback', 'ZiMenuFinalize']:
        entry = getattr(native, name)
        entry.argtypes = [ctypes.c_uint]
        entry.restype = ctypes.c_uint
        assert entry(0) == 1603
    assert observe()['entries'] == [] and not any(read_tree(p) for p in MENU_ROOTS)
    msi_api = ctypes.WinDLL('msi')
    for name, parameters in [('MsiInstallProductW', [ctypes.c_wchar_p, ctypes.c_wchar_p]),
                             ('MsiConfigureProductExW', [ctypes.c_wchar_p, ctypes.c_int, ctypes.c_int, ctypes.c_wchar_p]),
                             ('MsiEnableLogW', [ctypes.c_uint, ctypes.c_wchar_p, ctypes.c_uint]),
                             ('MsiSetInternalUI', [ctypes.c_uint, ctypes.c_void_p])]:
        function = getattr(msi_api, name)
        function.argtypes = parameters
        function.restype = ctypes.c_uint
    msi_api.MsiSetInternalUI(2, None)
    default = Path(os.environ['USERPROFILE']) / '.zi-devtools/services.yml'
    assert not default.exists(), 'Disposable default config must be absent'
    known_ids = set()
    proofs = []
    counter = 0
    portable_config = folder / 'portable-fixture.yml'
    portable_config.write_text('services: {}\n')

    def portable_active():
        ledger = observe()
        assert len(ledger['entries']) == 1
        record = ledger['entries'][0]['installation']
        assert record['identity']['kind'] == 'portable'
        assert ledger['active'] == record['identity']
        assert all(read_tree(root)['']['zicodeinstallgeneration'][0] == record['generation'] for root in MENU_ROOTS)

    def invoke(label, options='', repair=False, uninstall=False, cleanup=False):
        nonlocal counter
        counter += 1
        log = folder / f'{counter:02d}-{label}.log'
        # Exclude INSTALLLOGMODE_LOGONLYONERROR (0x4000): successful operations
        # also need evidence. Flush each line before collecting the closed log.
        assert msi_api.MsiEnableLogW(0x3FFF, str(log), 2) == 0
        code = packages[label][1]['product_code']
        common = 'REBOOT=ReallySuppress MSIRESTARTMANAGERCONTROL=Disable '
        try:
            if uninstall or repair:
                state = 2 if uninstall else 5
                result = msi_api.MsiConfigureProductExW(code, 0, state, common + options)
            else:
                result = msi_api.MsiInstallProductW(str(packages[label][0]),
                    common + f'INSTALLFOLDER="{target}" ' + options)
        finally:
            msi_api.MsiEnableLogW(0, None, 0)
        if not log.is_file():
            raise RuntimeError(f'Installer returned {result}; expected diagnostic log absent')
        raw = log.read_bytes()
        encoding = 'utf-16' if raw.startswith((b'\xff\xfe', b'\xfe\xff')) else 'mbcs'
        text = raw.decode(encoding, errors='replace')
        text = re.sub(r'S-1-(?:\d+-)*\d+', '[redacted-user-identifier]', text)
        text = re.sub(r'(?im)^(.*Property\([A-Z]\):\s+[^=]*(?:password|token|cookie|secret|credential)[^=]*=).*$',
                      r'\1 [redacted]', text)
        # Only this sanitized copy is uploaded; raw Installer logs stay ephemeral.
        log.with_suffix('.safe.log').write_text(text, encoding='utf-8')
        for relative in read_tree(RECOVERY_ROOT):
            if relative:
                value = relative.removeprefix('\\')
                assert str(uuid.UUID(value)) == value
                known_ids.add(value)
        if result == 0 and not cleanup:
            assert not observe().get('pending_msi'), 'Successful MSI must finalize without observer assistance'
            if 'ZiMenu: applied' in text:
                assert 'ZiMenu: completed after InstallFinalize' in text
        return result, text

    def installed(label):
        entries = [entry for entry in observe()['entries'] if entry['installation']['identity']['kind'] == 'msi']
        assert len(entries) == 1
        record = entries[0]['installation']
        assert record['identity'] == {'kind': 'msi'}
        assert record['version'] == packages[label][1]['version']
        assert hashlib.sha256((target / 'ZiDevTools.exe').read_bytes()).hexdigest() == record['executable_sha256']
        assert (target / 'ZiDevTools.exe').read_bytes() == packages[label][2]
        for root in MENU_ROOTS:
            values = read_tree(root)['']
            assert values['zicodeowner'][0] == 'com.zicode.devtools'
            assert values['zicodemenuprotocol'][0] == '2'
            assert values['zicodeinstallgeneration'][0] == record['generation']
            assert values['zicodebinarysha256'][0] == record['executable_sha256']
        assert not observe().get('pending_msi') and not default.exists()
        return record
    try:
        assert invoke('old', 'ZIMENU=0')[0] == 0
        assert observe()['entries'] == [] and not any(read_tree(p) for p in MENU_ROOTS)
        assert invoke('old', uninstall=True)[0] == 0
        proofs.append('disabled first install and uninstall leave menus absent')
        assert invoke('old', 'ZIMENU=1 RollbackDisabled=1')[0] == 1603
        assert observe()['entries'] == [] and not any(read_tree(p) for p in MENU_ROOTS)
        observe('--portable', portable_config)
        portable_active()
        assert invoke('old', 'ZIMENU=1')[0] == 0
        before = installed('old')
        marker = target / 'user-data-must-survive.txt'
        marker.write_bytes(b'not installer owned')
        assert invoke('old', 'REINSTALL=ALL REINSTALLMODE=amus', repair=True)[0] == 0
        repaired = installed('old')
        assert repaired['generation'] != before['generation']
        proofs.append('enabled first install and inherited repair use DLL and real files')
        for label in ['before', 'after']:
            menus = [read_tree(p) for p in MENU_ROOTS]
            result, log = invoke(label)
            assert result == 1603 and 'ZiMenu: reverted' in log
            assert 'ZiMenu: completed after InstallFinalize' not in log, 'Failed MSI must retain rollback until restoration'
            if label == 'after':
                assert 'ZiMenu: applied' in log, 'Failure must be after actual deferred menu execution'
            assert installed('old') == repaired
            assert [read_tree(p) for p in MENU_ROOTS] == menus
            assert marker.read_bytes() == b'not installer owned'
            proofs.append(label + ' failure restores old file, menu generation and record')
        assert invoke('new')[0] == 0
        installed('new')
        assert invoke('new', 'ZIMENU=0 REINSTALL=ALL REINSTALLMODE=amus', repair=True)[0] == 0
        portable_active()
        assert invoke('new', 'ZIMENU=1 REINSTALL=ALL REINSTALLMODE=amus', repair=True)[0] == 0
        installed('new')
        assert invoke('new', uninstall=True)[0] == 0
        portable_active()
        observe('--remove-portable', portable_config)
        assert observe()['entries'] == [] and not any(read_tree(p) for p in MENU_ROOTS)
        assert not (target / 'ZiDevTools.exe').exists() and marker.exists() and not default.exists()
        proofs.append('inherited upgrade, explicit disable/enable and uninstall preserve user data')
        assert invoke('new', 'ZIMENU=1')[0] == 0
        installed('new')
        observe('--portable', portable_config)
        observe('--remove-portable', portable_config)
        installed('new')
        assert invoke('new', uninstall=True)[0] == 0
        assert observe()['entries'] == [] and not any(read_tree(p) for p in MENU_ROOTS)
        proofs.append('both portable/MSI removal orders preserve the remaining native record and alias')
        for operation_id in known_ids:
            observe('--forget', operation_id)
        assert read_tree(RECOVERY_ROOT) == {'': {}}
        (folder / 'proof.json').write_text(json.dumps(dict(passed=proofs, productionInstallerWired=False,
            automaticCompletion=True, scope='real candidate DLL/MSI; immediate finalizer after InstallFinalize, no observer completion'), indent=2) + '\n')
        print('PASS: candidate DLL MSI transactions with automatic post-InstallFinalize completion')
    finally:
        for label in packages:
            # Exact fixture product IDs only. On a failed test preserve journal/menu
            # evidence but skip our candidate hook so fixture package cleanup can run.
            code = packages[label][1]['product_code']
            result, _ = invoke(label, 'UPGRADINGPRODUCTCODE=' + code, uninstall=True, cleanup=True)
            assert result in (0, 1605)
        assert observe('--products') == []


if __name__ == '__main__':
    main()
