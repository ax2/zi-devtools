"""Actual MSI transactions, only on disposable GitHub-hosted Windows runners.

Uses public packaging/windows.wxs with tiny non-executable fixture payloads;
never installs our published product on the developer workstation.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import subprocess
import tempfile
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
NS = '{http://wixtoolset.org/schemas/v4/wxs}'
ET.register_namespace('', NS[1:-1])


def main():
    if not __debug__:
        raise RuntimeError('Optimized Python disables validation; installer fixture refused')
    parser = argparse.ArgumentParser()
    parser.add_argument('--compile-only', action='store_true', help='Build fixtures without installing')
    args = parser.parse_args()
    probe = ROOT / 'target/debug/examples/inspect_installer.exe'
    assert probe.is_file(), 'Build the read-only probe first'
    wix = os.environ.get('ZIDEVTOOLS_WIX', 'wix')
    # Explicitly refuse self-hosted or local execution for install/uninstall.
    if not args.compile_only:
        assert os.environ.get('GITHUB_ACTIONS') == 'true' and os.environ.get('CI') == 'true'
        assert os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted', 'Disposable runner required'
        assert probe_json(probe) == [], 'Existing Zi DevTools product detected; refuse all mutation'
    base = Path(os.environ['RUNNER_TEMP']) if not args.compile_only else ROOT / 'target'
    folder = Path(tempfile.mkdtemp(prefix='zi-msi-transaction-', dir=base)).resolve()
    assert folder.is_relative_to(base.resolve()) and not folder.is_symlink()
    target = folder / '安装 & upgrade with spaces'
    portable = folder / 'portable'
    portable.mkdir()
    packages = {}
    for label, version, fail in [('old', '0.1.0', False), ('failure', '0.2.0', True), ('new', '0.2.0', False)]:
        payload = folder / label
        payload.mkdir()
        for name in ['ZiDevTools.exe', 'ZiDevToolsMcp.exe']:
            (payload / name).write_bytes(f'fixture {label} {name}\n'.encode())
        tree = ET.parse(ROOT / 'packaging/windows.wxs')
        package = tree.getroot().find(NS + 'Package')
        package.set('Version', version)
        if label == 'old':
            package.find(NS + 'MajorUpgrade').set('Schedule', 'afterInstallValidate')
            package.remove(package.find(NS + 'SetProperty'))  # legacy: no ARP install location
        for file in package.iter(NS + 'File'):
            if file.get('Id') in ['ZiDevToolsExe', 'ZiDevToolsMcpExe']:
                file.set('Source', str(payload / Path(file.get('Source')).name))
            else:
                file.set('Source', str(ROOT / 'LICENSE'))
        if fail:
            ET.SubElement(package, NS + 'CustomAction', Id='FixtureFailure', Error='Intentional rollback fixture')
            seq = ET.SubElement(package, NS + 'InstallExecuteSequence')
            ET.SubElement(seq, NS + 'Custom', Action='FixtureFailure', Before='InstallFinalize', Condition='NOT REMOVE')
        source = folder / f'{label}.wxs'
        tree.write(source, encoding='utf-8', xml_declaration=True)
        msi = folder / f'{label}.msi'
        subprocess.run([wix, 'build', str(source), '-arch', 'x64', '-o', str(msi)], check=True, cwd=ROOT, timeout=180)
        metadata = probe_json(probe, msi)
        assert metadata['version'] == version and metadata['architecture'].startswith('x64;')
        assert metadata['transactional_upgrade'] == (label != 'old')
        assert metadata['records_install_location'] == (label != 'old')
        packages[label] = (msi, metadata)
    assert len({m['desktop_component'] for _, m in packages.values()}) == 1, 'Desktop component identity must stay stable'
    print('PASS: all three real WiX fixtures compiled and inspected read-only', flush=True)
    if args.compile_only:
        return

    # Native installer APIs accept a property string, without a shell or msiexec quoting.
    dll = ctypes.WinDLL('msi')
    dll.MsiSetInternalUI.argtypes = [ctypes.c_uint, ctypes.c_void_p]
    dll.MsiSetInternalUI.restype = ctypes.c_uint
    dll.MsiInstallProductW.argtypes = [ctypes.c_wchar_p, ctypes.c_wchar_p]
    dll.MsiInstallProductW.restype = ctypes.c_uint
    dll.MsiConfigureProductExW.argtypes = [ctypes.c_wchar_p, ctypes.c_int, ctypes.c_int, ctypes.c_wchar_p]
    dll.MsiConfigureProductExW.restype = ctypes.c_uint
    dll.MsiEnableLogW.argtypes = [ctypes.c_uint, ctypes.c_wchar_p, ctypes.c_uint]
    dll.MsiEnableLogW.restype = ctypes.c_uint
    dll.MsiSetInternalUI(2, None)  # NONE, test runner only

    def install(label):
        log = folder / f'install-{label}.log'
        assert dll.MsiEnableLogW(0xFFFF, str(log), 0) == 0
        result = dll.MsiInstallProductW(str(packages[label][0]),
            f'INSTALLFOLDER="{target}" REBOOT=ReallySuppress MSIRESTARTMANAGERCONTROL=Disable')
        dll.MsiEnableLogW(0, None, 0)
        print(f'INSTALL {label}: {result}', flush=True)
        return result

    def verify(label):
        version = packages[label][1]['version']
        expected_code = packages[label][1]['product_code']
        products = probe_json(probe)
        assert len(products) == 1 and products[0]['version'] == version and products[0]['product_code'] == expected_code, products
        assert Path(products[0]['executable']).parent.resolve() == target.resolve()
        if label == 'old':
            assert products[0]['install_location'] is None, 'Legacy package must lack InstallLocation'
        else:
            assert Path(products[0]['install_location']).resolve() == target.resolve(), 'ARP install location must match custom path'
        for name in ['ZiDevTools.exe', 'ZiDevToolsMcp.exe']:
            assert (target / name).read_bytes() == (folder / label / name).read_bytes(), f'{label} {name}'
        denied = subprocess.run([probe, '--portable-directory', target], capture_output=True, timeout=30)
        assert denied.returncode != 0, 'MSI directory must refuse portable replacement'
        subprocess.run([probe, '--portable-directory', portable], check=True, timeout=30)

    try:
        assert install('old') == 0
        verify('old')
        marker = target / 'user-data-must-survive.txt'
        marker.write_bytes(b'not installer owned')
        assert install('failure') == 1603, 'Intentional upgrade must fail'
        verify('old')
        assert marker.read_bytes() == b'not installer owned'
        print('PASS: failed major upgrade restored old product and both EXEs; user data retained', flush=True)
        assert install('new') == 0
        verify('new')
        assert marker.read_bytes() == b'not installer owned'
        print('PASS: successful upgrade, native ownership, installed/portable coexistence', flush=True)
    finally:
        for _, metadata in packages.values():
            # Only product codes created by this fixture; never uninstall by guessed path.
            result = dll.MsiConfigureProductExW(metadata['product_code'], 0, 2, 'REBOOT=ReallySuppress')
            assert result in (0, 1605), f'Fixture uninstall returned {result}'
    assert probe_json(probe) == []
    assert not (target / 'ZiDevTools.exe').exists() and not (target / 'ZiDevToolsMcp.exe').exists()
    assert marker.read_bytes() == b'not installer owned'
    print('PASS: fixture uninstall removed owned EXEs and registration, preserved user data', flush=True)


def probe_json(probe, *args):
    result = subprocess.run([probe, *args], check=True, capture_output=True, timeout=30)
    return json.loads(result.stdout)


if __name__ == '__main__':
    main()
