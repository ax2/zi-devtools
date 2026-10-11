"""Candidate DLL inside real MSI transactions on a disposable Windows runner.

Production and candidates share the menu action include. Ordinary candidates finalize automatically;
the receipt-only candidate exercises the explicit desktop recovery backend.
The separate outer-transaction probe must prove rollback before production wiring.
"""
import argparse
import ctypes
import hashlib
import json
import os
import re
import socket
from pathlib import Path
import subprocess
import tempfile
import time
import urllib.request
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


def sanitize_log(log):
    raw = log.read_bytes()
    encoding = 'utf-16' if raw.startswith((b'\xff\xfe', b'\xfe\xff')) else 'mbcs'
    text = raw.decode(encoding, errors='replace')
    text = re.sub(r'S-1-(?:\d+-)*\d+', '[redacted-user-identifier]', text)
    text = re.sub(r'(?im)^(.*Property\([A-Z]\):\s+[^=]*(?:password|token|cookie|secret|credential)[^=]*=).*$',
                  r'\1 [redacted]', text)
    log.with_suffix('.safe.log').write_text(text, encoding='utf-8')
    return text


def main():
    if not __debug__:
        raise RuntimeError('Assertions required')
    parser = argparse.ArgumentParser()
    parser.add_argument('--compile-only', action='store_true')
    parser.add_argument('--outer-transaction-probe', action='store_true')
    parser.add_argument('--outer-commit-probe', action='store_true')
    parser.add_argument('--real-payload', action='store_true')
    args = parser.parse_args()
    assert not (args.real_payload and (args.outer_transaction_probe or args.outer_commit_probe))
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
                                   ('after', '0.2.0', 'after'), ('new', '0.2.0', None),
                                   ('receipt', '0.3.0', None), ('commitfail', '0.2.0', 'commit'),
                                   ('preparefail', '0.2.0', 'prepared'), ('restore', '0.2.0', 'restore')]:
        payload = folder / label
        payload.mkdir()
        # Synthetic MZ payloads satisfy the product's documented basic check;
        # never execute them or claim full PE/Shell/UI acceptance.
        content = ((build / 'debug/ZiDevTools.exe').read_bytes()
                   if args.real_payload and label != 'old'
                   else b'MZ' + (b'old' if label == 'old' else b'new') * 64)
        for name in ['ZiDevTools.exe', 'ZiDevToolsMcp.exe']:
            data = ((build / 'debug' / name).read_bytes()
                    if args.real_payload and label != 'old' else content)
            (payload / name).write_bytes(data)
        tree = ET.parse(ROOT / 'packaging/windows.wxs')
        package = tree.getroot().find(NS + 'Package')
        package.set('Version', version)
        # Rollback CA itself generates script, so REP cannot follow it in the
        # afterInstallInitialize slot (MSI error 2613). Install replacement files
        # first with stable component GUIDs, then remove old clients inside the
        # same transaction. File rollback precedes our early rollback callback.
        assert package.find(NS + 'MajorUpgrade').get('Schedule') == 'afterInstallExecute'
        for file in package.iter(NS + 'File'):
            file.set('Source', str(payload / Path(file.get('Source')).name)
                     if file.get('Id') in ['ZiDevToolsExe', 'ZiDevToolsMcpExe'] else str(ROOT / 'LICENSE'))
        # ET deliberately omits processing instructions. Expand the exact
        # production include, then modify only explicit fault-injection points.
        shared = ET.parse(ROOT / 'packaging/explorer.wxi').getroot()
        for child in shared:
            package.append(child)
        package.find(NS + "Property[@Id='ZiMenuBinarySha256']").set('Value', hashlib.sha256(content).hexdigest())
        package.find(NS + "Binary[@Id='ZiMenuActions']").set('SourceFile', str(actions))
        ET.SubElement(package, NS + 'Property', Id='ZIFIXTURE_DISABLE_ROLLBACK', Secure='yes')
        sequence = package.find(NS + 'InstallExecuteSequence')
        if label == 'receipt':
            package.remove(package.find(NS + "CustomAction[@Id='ZiMenuFinalize']"))
            sequence.remove(sequence.find(NS + "Custom[@Action='ZiMenuFinalize']"))
        # RollbackDisabled is an engine-owned private property, not a public
        # command-line switch. Exercise the actual standard action, per package;
        # never change a machine/user Installer policy on the runner.
        ET.SubElement(sequence, NS + 'DisableRollback', Before='InstallInitialize',
                      Condition='ZIFIXTURE_DISABLE_ROLLBACK = "1"')
        # Queue menu rollback before file writes so reverse execution restores
        # files before verifying the old executable and restoring its menu.
        if label == 'restore':
            # Model a missing menu callback, not an actual process kill. Native
            # Installer still rolls back its files and product registration.
            sequence.remove(sequence.find(NS + "Custom[@Action='ZiMenuRollback']"))
            prepare = sequence.find(NS + "Custom[@Action='ZiMenuPrepare']")
            del prepare.attrib['Before']
            prepare.set('After', 'InstallInitialize')
        if fault:
            if fault == 'before':
                # Fail in the real execution script before file writes. An
                # immediate Type-19 error here aborts script construction before
                # the engine has registered rollback callbacks at all. Empty
                # CustomActionData makes the strict production DLL reject safely.
                ET.SubElement(package, NS + 'CustomAction', Id='MenuFixtureFailure',
                              BinaryRef='ZiMenuActions', DllEntry='ZiMenuExecute',
                              Execute='deferred', Return='check', Impersonate='yes')
            elif fault == 'commit':
                ET.SubElement(package, NS + 'CustomAction', Id='MenuFixtureFailure',
                              BinaryRef='ZiMenuActions', DllEntry='ZiMenuExecute',
                              Execute='commit', Return='check', Impersonate='yes')
            else:
                ET.SubElement(package, NS + 'CustomAction', Id='MenuFixtureFailure', Error='Intentional menu rollback fixture')
            position = ({'Sequence': '1520'} if fault in ['before', 'prepared'] else
                        {'After': 'ZiMenuCommitReady'} if fault == 'commit' else {'Before': 'InstallFinalize'})
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
                    'InstallFiles', 'ZiMenuExecute', 'ZiMenuCommitReady', 'InstallExecute', 'RemoveExistingProducts',
                    'InstallFinalize', 'ZiMenuCommitReceipt']
        if label == 'restore':
            required.remove('ZiMenuRollback')
            assert 'ZiMenuRollback' not in order
        if label != 'receipt':
            required.append('ZiMenuFinalize')
        positions = [order[name] for name in required]
        assert all(a < b for a, b in zip(positions, positions[1:])), dict(zip(required, positions))
        if fault in ['before', 'prepared']:
            assert order['ZiMenuRollback'] < order['MenuFixtureFailure'] < order['InstallFiles']
        if fault in ['after', 'restore']:
            assert order['ZiMenuExecute'] < order['InstallExecute'] < order['RemoveExistingProducts'] < order['MenuFixtureFailure'] < order['InstallFinalize']
        if fault == 'commit':
            assert order['ZiMenuCommitReady'] < order['MenuFixtureFailure'] < order['InstallExecute']
        compile_orders[label] = dict(zip(required, positions))
        packages[label] = (msi, metadata, content)
    assert len({value[1]['desktop_component'] for value in packages.values()}) == 1
    (folder / 'compile-proof.json').write_text(json.dumps(dict(sequences=compile_orders,
        dllSha256=hashlib.sha256(actions.read_bytes()).hexdigest()), indent=2) + '\n')
    if args.compile_only:
        print('PASS: eight candidate DLL fixtures compiled, no installation performed')
        return

    native = ctypes.WinDLL(str(actions))
    for name in ['ZiMenuPrepare', 'ZiMenuExecute', 'ZiMenuRollback', 'ZiMenuFinalize', 'ZiMenuCommitReceipt', 'ZiMenuCommitReady']:
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
        # Only this sanitized copy is uploaded; raw Installer logs stay ephemeral.
        text = sanitize_log(log)
        for relative in read_tree(RECOVERY_ROOT):
            if relative:
                value = relative.removeprefix('\\')
                assert str(uuid.UUID(value)) == value
                known_ids.add(value)
        if result == 0 and not cleanup:
            if label == 'receipt':
                pending = observe().get('pending_msi')
                assert pending and any(record == dict(id=pending, status='Committed') for record in observe('--journal'))
                assert 'ZiMenu: committed receipt after InstallFinalize' in text
                assert 'ZiMenu: completed after InstallFinalize' not in text
            else:
                assert not observe().get('pending_msi'), 'Successful MSI must finalize without observer assistance'
            if label != 'receipt' and 'ZiMenu: applied' in text:
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
        if args.outer_transaction_probe or args.outer_commit_probe:
            # A package-level InstallFinalize is not evidence that an owning
            # multi-package transaction has committed. Observe before AND after
            # the real outer rollback; never repair the state to make this pass.
            begin = msi_api.MsiBeginTransactionW
            begin.argtypes = [ctypes.c_wchar_p, ctypes.c_uint,
                              ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_void_p)]
            begin.restype = ctypes.c_uint
            end = msi_api.MsiEndTransaction
            end.argtypes, end.restype = [ctypes.c_uint], ctypes.c_uint
            handle, event = ctypes.c_uint(), ctypes.c_void_p()
            baseline = observe()
            baseline_menus = [read_tree(p) for p in MENU_ROOTS]
            assert begin('ZiDevTools isolated outer rollback probe', 0,
                         ctypes.byref(handle), ctypes.byref(event)) == 0
            try:
                result, _ = invoke('old', 'ZIMENU=1', cleanup=True)
                during = observe()
                during_journal = observe('--journal')
            finally:
                rollback_log = folder / 'outer-rollback.log'
                log_result = msi_api.MsiEnableLogW(0x3FFF, str(rollback_log), 2)
                try:
                    # End the transaction even if diagnostics could not start.
                    rollback_result = end(1 if args.outer_commit_probe else 0)
                finally:
                    msi_api.MsiEnableLogW(0, None, 0)
                    msi_api.MsiCloseHandle(handle)
                    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
                    kernel.CloseHandle.argtypes = [ctypes.c_void_p]
                    kernel.CloseHandle.restype = ctypes.c_int
                    if event.value:
                        assert kernel.CloseHandle(event)
                assert log_result == 0
                sanitize_log(rollback_log)
            after = observe()
            after_menus = [read_tree(p) for p in MENU_ROOTS]
            deferred = (during.get('pending_msi') is not None
                        and during_journal == [dict(id=during['pending_msi'], status='Applied')])
            if args.outer_commit_probe:
                pending = after.get('pending_msi')
                assert result == 0 and rollback_result == 0 and deferred and pending
                assert observe('--journal') == [dict(id=pending, status='CommitReady')]
                rejected = subprocess.run([str(probe), '--resume', pending], capture_output=True, timeout=30)
                assert rejected.returncode != 0 and observe() == after
                # This callback belongs to the actual transaction owner, only
                # AFTER successful MsiEndTransaction(COMMIT), never a CA or UI.
                observe('--owner-commit', pending)
                observe('--resume', pending)
                installed('old')
                assert invoke('old', uninstall=True)[0] == 0
                assert observe()['entries'] == [] and not any(read_tree(p) for p in MENU_ROOTS)
                for operation_id in known_ids:
                    observe('--forget', operation_id)
                (folder / 'proof.json').write_text(json.dumps(dict(
                    scope='real outer commit; explicit owner completion after API return',
                    installResult=result, commitResult=rollback_result,
                    deferredBeforeOuterEnd=deferred, readyCannotResume=True,
                    ownerCompletion=True, productionInstallerWired=False), indent=2) + '\n')
                print('PASS: outer commit preserves Undo until explicit transaction owner completion')
                return
            # Ledger cursors may advance on a correct rollback; compare actual
            # registrations, active selection, reservation, files and products.
            restored = (after['entries'] == baseline['entries']
                        and after.get('active') == baseline.get('active')
                        and after.get('pending_msi') == baseline.get('pending_msi')
                        and after_menus == baseline_menus
                        and not (target / 'ZiDevTools.exe').exists()
                        and observe('--products') == [])
            (folder / 'proof.json').write_text(json.dumps(dict(
                scope='real MsiBeginTransaction package install then outer rollback',
                installResult=result, rollbackResult=rollback_result,
                journalBeforeOuterEnd=during_journal,
                pendingBeforeOuterEnd=during.get('pending_msi'),
                journalAfterOuterEnd=observe('--journal'),
                remainingRegistrations=len(after['entries']),
                remainingMenus=[bool(tree) for tree in after_menus],
                remainingFiles=(target / 'ZiDevTools.exe').exists(),
                remainingProducts=observe('--products'),
                pendingAfterOuterEnd=after.get('pending_msi'),
                restored=restored, productionInstallerWired=False), indent=2) + '\n')
            assert result == 0 and rollback_result == 0 and deferred and restored, \
                'Outer rollback did not restore the menu lifecycle; production wiring remains blocked'
            print('PASS: outer MSI rollback restored files, products, menus and registrations')
            return
        assert invoke('old', 'ZIMENU=0')[0] == 0
        assert observe()['entries'] == [] and not any(read_tree(p) for p in MENU_ROOTS)
        assert invoke('old', uninstall=True)[0] == 0
        proofs.append('disabled first install and uninstall leave menus absent')
        result, log = invoke('old', 'ZIMENU=1 ZIFIXTURE_DISABLE_ROLLBACK=1')
        assert result == 1603 and 'RollbackDisabled' in log and 'ZiMenu: prepared' not in log
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
        menus = [read_tree(p) for p in MENU_ROOTS]
        result, log = invoke('preparefail')
        assert result == 1603 and 'ZiMenu: prepared' in log and 'ZiMenu: applied' not in log
        prepared = [r for r in observe('--journal') if r['status'] == 'Prepared']
        assert len(prepared) == 1 and installed('old') == repaired
        # No execution script ran. Cancel only the unexecuted menu record;
        # never claim that this action rolls back Installer-owned files.
        observe('--cancel-prepared', prepared[0]['id'])
        assert any(r == dict(id=prepared[0]['id'], status='Canceled') for r in observe('--journal'))
        assert installed('old') == repaired and [read_tree(p) for p in MENU_ROOTS] == menus
        proofs.append('immediate preparation failure leaves a Prepared record; explicit cancellation preserves old files, menus and registrations')
        result, log = invoke('restore')
        assert result == 1603 and 'ZiMenu: applied' in log and 'ZiMenu: reverted' not in log
        pending = observe()['pending_msi']
        assert any(r == dict(id=pending, status='Applied') for r in observe('--journal'))
        assert (target / 'ZiDevTools.exe').read_bytes() == packages['old'][2]
        products = observe('--products')
        assert len(products) == 1 and products[0]['product_code'] == packages['old'][1]['product_code']
        observe('--restore-menu', pending)
        assert installed('old') == repaired and [read_tree(p) for p in MENU_ROOTS] == menus
        assert any(r == dict(id=pending, status='Reverted') for r in observe('--journal'))
        proofs.append('missing menu rollback callback recovers through verified desktop restoration after actual old files and MSI ownership return')
        for label in ['before', 'after', 'commitfail']:
            menus = [read_tree(p) for p in MENU_ROOTS]
            result, log = invoke(label)
            assert result == 1603 and 'ZiMenu: reverted' in log
            assert 'ZiMenu: completed after InstallFinalize' not in log, 'Failed MSI must retain rollback until restoration'
            if label == 'after':
                assert 'ZiMenu: applied' in log, 'Failure must be after actual deferred menu execution'
            if label == 'commitfail':
                assert 'ZiMenu: commit ready; Undo retained' in log
            assert installed('old') == repaired
            assert [read_tree(p) for p in MENU_ROOTS] == menus
            assert marker.read_bytes() == b'not installer owned'
            proofs.append(label + ' failure restores old file, menu generation and record')
        assert invoke('new')[0] == 0
        installed('new')
        if args.real_payload:
            # Execute the installed product, not a build-tree substitute.
            assert (target / 'ZiDevToolsMcp.exe').read_bytes() == (build / 'debug/ZiDevToolsMcp.exe').read_bytes()
            with socket.socket() as reserve:
                reserve.bind(('127.0.0.1', 0))
                port = reserve.getsockname()[1]
            exe = target / 'ZiDevTools.exe'
            config = folder / 'disposable-service.yml'
            with (folder / 'installed-service.safe.log').open('wb') as output:
                process = subprocess.Popen([str(exe), '--fixture-server', str(port), '--config', str(config), '--no-restore'],
                                           stdout=output, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
                try:
                    client = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                    deadline = time.monotonic() + 20
                    while True:
                        assert process.poll() is None, 'Installed fixture exited before startup'
                        try:
                            with client.open(f'http://127.0.0.1:{port}/health', timeout=1) as response:
                                assert json.load(response) == {'status': 'ok', 'source': 'zi-devtools-fixture'}
                            break
                        except OSError:
                            assert time.monotonic() < deadline, 'Installed fixture startup timeout'
                            time.sleep(.1)
                    subprocess.run([str(exe), '--fixture-shutdown', str(port), '--config', str(config), '--no-restore'],
                                   check=True, timeout=10, creationflags=subprocess.CREATE_NO_WINDOW)
                    assert process.wait(timeout=10) == 0
                    with socket.socket() as released:
                        released.bind(('127.0.0.1', port))
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait(timeout=10)
            proofs.append('installed real desktop EXE starts and stops disposable service; both installed EXEs match build hashes')
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
        # The receipt-only package models interrupted finalization. The native
        # API has returned; a fresh observer uses the desktop recovery backend,
        # not the old unconditional --finish helper, to complete this operation.
        assert invoke('receipt', 'ZIMENU=1')[0] == 0
        pending = observe()['pending_msi']
        menus = [read_tree(p) for p in MENU_ROOTS]
        before = observe()['entries']
        observe('--resume', pending)
        assert observe()['entries'] == before and [read_tree(p) for p in MENU_ROOTS] == menus
        installed('receipt')
        assert invoke('receipt', uninstall=True)[0] == 0
        observe('--resume', observe()['pending_msi'])
        assert observe()['entries'] == [] and not any(read_tree(p) for p in MENU_ROOTS)
        proofs.append('fresh-process receipt-backed recovery completes interrupted install and uninstall without changing menus or entries')
        for operation_id in known_ids:
            observe('--forget', operation_id)
        assert read_tree(RECOVERY_ROOT) == {'': {}}
        (folder / 'proof.json').write_text(json.dumps(dict(passed=proofs, productionTemplateWired=True,
            realCurrentPayload=args.real_payload, legacyReleaseUpgradeVerified=False,
            automaticCompletion=True, receiptRecovery=True,
            scope='ordinary candidates auto-complete; receipt-only candidate resumes via guarded desktop backend after native API return'), indent=2) + '\n')
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
