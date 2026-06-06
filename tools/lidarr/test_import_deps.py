import io
import os
import shutil
import sys
import tarfile
import tempfile
import unittest
import zipfile
from argparse import Namespace

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import ImportLidarrManual as mod  # noqa: E402


class TestDetectPlatform(unittest.TestCase):
    def test_mac(self):
        self.assertEqual(mod.detect_platform(platform='darwin'), 'mac')

    def test_unraid(self):
        with tempfile.NamedTemporaryFile() as marker:
            self.assertEqual(
                mod.detect_platform(platform='linux', unraid_marker=marker.name),
                'unraid',
            )

    def test_linux(self):
        self.assertEqual(
            mod.detect_platform(platform='linux2', unraid_marker='/no/such/marker'),
            'linux',
        )

    def test_unknown(self):
        self.assertEqual(
            mod.detect_platform(platform='win32', unraid_marker='/no/such/marker'),
            'unknown',
        )

    def test_unraid_via_kernel_release_without_marker(self):
        # No marker file, but the Unraid kernel suffix is the signal.
        self.assertEqual(
            mod.detect_platform(platform='linux', unraid_marker='/no/such/marker',
                                kernel_release='6.18.33-Unraid'),
            'unraid',
        )

    def test_plain_linux_kernel_is_not_unraid(self):
        self.assertEqual(
            mod.detect_platform(platform='linux', unraid_marker='/no/such/marker',
                                kernel_release='5.15.0-generic'),
            'linux',
        )


class TestBuildDependencies(unittest.TestCase):
    def test_table_has_expected_deps(self):
        names = {d.name for d in mod.build_dependencies()}
        self.assertEqual(
            names,
            {'mutagen', 'essentia', 'cv2', 'magick', 'ffmpeg', 'rsgain', 'git'},
        )

    def test_kinds_and_optional_flags(self):
        by_name = {d.name: d for d in mod.build_dependencies()}
        self.assertEqual(by_name['mutagen'].kind, 'pip')
        self.assertEqual(by_name['magick'].kind, 'binary')
        self.assertTrue(by_name['cv2'].optional)
        self.assertTrue(by_name['git'].optional)
        self.assertFalse(by_name['rsgain'].optional)

    def test_pip_packages_present(self):
        by_name = {d.name: d for d in mod.build_dependencies()}
        self.assertEqual(by_name['essentia'].packages['pip'], 'essentia')
        self.assertEqual(by_name['magick'].packages['brew'], 'imagemagick')


class TestScanArtworkKinds(unittest.TestCase):
    def _album(self, files):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        for name in files:
            open(os.path.join(d, name), 'w').close()
        return d

    def test_detects_convertible(self):
        d = self._album(['folder.tif', 'track01.flac'])
        scan = mod.scan_artwork_kinds([d])
        self.assertTrue(scan['convertible'])
        self.assertFalse(scan['animated'])

    def test_detects_animated(self):
        d = self._album(['folder.mp4', 'track01.flac'])
        scan = mod.scan_artwork_kinds([d])
        self.assertTrue(scan['animated'])

    def test_plain_jpg_triggers_nothing(self):
        d = self._album(['folder.jpg', 'track01.flac'])
        scan = mod.scan_artwork_kinds([d])
        self.assertFalse(scan['convertible'])
        self.assertFalse(scan['animated'])


class TestNeededDependencies(unittest.TestCase):
    def setUp(self):
        # Pretend nothing is installed so every dep is "missing".
        self.deps = [
            mod.Dependency('mutagen', 'pip', lambda: False, 'x', {'pip': 'mutagen'},
                           needed_when=lambda a, s: not a.skip_bpm),
            mod.Dependency('rsgain', 'binary', lambda: False, 'x',
                           {'brew': 'rsgain'},
                           needed_when=lambda a, s: not a.no_rsgain),
            mod.Dependency('ffmpeg', 'binary', lambda: False, 'x',
                           {'brew': 'ffmpeg'},
                           needed_when=lambda a, s: s['animated']),
            mod.Dependency('git', 'binary', lambda: False, 'x',
                           {'brew': 'git'}, optional=True),
        ]

    def test_skip_bpm_drops_mutagen(self):
        args = Namespace(skip_bpm=True, no_rsgain=False)
        scan = {'convertible': False, 'animated': False}
        names = {d.name for d in mod.needed_dependencies(self.deps, args, scan)}
        self.assertNotIn('mutagen', names)
        self.assertIn('rsgain', names)

    def test_no_rsgain_drops_rsgain(self):
        args = Namespace(skip_bpm=False, no_rsgain=True)
        scan = {'convertible': False, 'animated': False}
        names = {d.name for d in mod.needed_dependencies(self.deps, args, scan)}
        self.assertNotIn('rsgain', names)

    def test_no_animated_art_drops_ffmpeg(self):
        args = Namespace(skip_bpm=False, no_rsgain=False)
        scan = {'convertible': False, 'animated': False}
        names = {d.name for d in mod.needed_dependencies(self.deps, args, scan)}
        self.assertNotIn('ffmpeg', names)

    def test_optional_never_in_needed(self):
        args = Namespace(skip_bpm=False, no_rsgain=False)
        scan = {'convertible': True, 'animated': True}
        names = {d.name for d in mod.needed_dependencies(self.deps, args, scan)}
        self.assertNotIn('git', names)


class TestInstallCommandFor(unittest.TestCase):
    def setUp(self):
        self.by_name = {d.name: d for d in mod.build_dependencies()}

    def test_pip_uses_current_interpreter(self):
        cmd = mod.install_command_for(self.by_name['mutagen'], 'unraid')
        self.assertEqual(cmd, [sys.executable, '-m', 'pip', 'install', 'mutagen'])

    def test_binary_on_mac_uses_brew(self):
        cmd = mod.install_command_for(self.by_name['magick'], 'mac')
        self.assertEqual(cmd, ['brew', 'install', 'imagemagick'])

    def test_binary_on_unraid_uses_unget(self):
        cmd = mod.install_command_for(self.by_name['ffmpeg'], 'unraid')
        self.assertEqual(cmd, ['un-get', 'install', 'ffmpeg'])

    def test_binary_on_unknown_returns_none(self):
        cmd = mod.install_command_for(self.by_name['ffmpeg'], 'unknown')
        self.assertIsNone(cmd)


class TestInstallDependency(unittest.TestCase):
    def setUp(self):
        self.dep = mod.Dependency('ffmpeg', 'binary', lambda: False, 'x',
                                  {'un-get': 'ffmpeg'})
        self._orig_run = mod.subprocess.run

    def tearDown(self):
        mod.subprocess.run = self._orig_run

    def test_success(self):
        class R:
            returncode = 0
            stderr = ''
        mod.subprocess.run = lambda *a, **k: R()
        self.assertTrue(mod.install_dependency(self.dep, 'unraid'))

    def test_nonzero_returns_false(self):
        class R:
            returncode = 1
            stderr = 'package not found'
        mod.subprocess.run = lambda *a, **k: R()
        self.assertFalse(mod.install_dependency(self.dep, 'unraid'))

    def test_missing_manager_returns_false(self):
        def boom(*a, **k):
            raise FileNotFoundError()
        mod.subprocess.run = boom
        self.assertFalse(mod.install_dependency(self.dep, 'unraid'))

    def test_print_only_platform_returns_false(self):
        self.assertFalse(mod.install_dependency(self.dep, 'unknown'))


class TestWriteIdempotentBlock(unittest.TestCase):
    def test_writes_once_then_replaces(self):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        path = os.path.join(d, 'go')
        with open(path, 'w') as f:
            f.write('#!/bin/bash\necho existing\n')
        mod.write_idempotent_block(path, 'TESTMARK', 'echo one')
        mod.write_idempotent_block(path, 'TESTMARK', 'echo two')
        with open(path) as f:
            content = f.read()
        self.assertEqual(content.count('# >>> TESTMARK >>>'), 1)
        self.assertEqual(content.count('# <<< TESTMARK <<<'), 1)
        self.assertIn('echo two', content)
        self.assertNotIn('echo one', content)
        self.assertIn('echo existing', content)

    def test_creates_missing_file(self):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        path = os.path.join(d, 'go')
        mod.write_idempotent_block(path, 'TESTMARK', 'echo hi')
        with open(path) as f:
            content = f.read()
        self.assertIn('echo hi', content)

    def test_out_of_order_markers_appended_not_corrupted(self):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        path = os.path.join(d, 'go')
        with open(path, 'w') as f:
            f.write('# <<< TESTMARK <<<\nstray\n# >>> TESTMARK >>>\n')
        mod.write_idempotent_block(path, 'TESTMARK', 'echo ok')
        with open(path) as f:
            content = f.read()
        # A well-formed block is appended; nothing is doubled/corrupted.
        self.assertIn('# >>> TESTMARK >>>\necho ok\n# <<< TESTMARK <<<', content)


class TestWriteBootPersistence(unittest.TestCase):
    def test_prefers_user_scripts_when_dir_exists(self):
        us_dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, us_dir, True)
        go_dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, go_dir, True)
        go_file = os.path.join(go_dir, 'go')
        dest = mod.write_boot_persistence(
            ['mutagen', 'essentia'], '/mnt/vms/x/tools/ImportLidarrManual.py',
            user_scripts_dir=us_dir, go_file=go_file)
        self.assertTrue(dest.startswith(us_dir))
        with open(dest) as f:
            content = f.read()
        self.assertIn('#!/bin/bash', content)
        self.assertIn('pip install essentia mutagen', content)
        self.assertIn('ln -sf /mnt/vms/x/tools/ImportLidarrManual.py /usr/local/bin/il',
                      content)
        self.assertFalse(os.path.exists(go_file))

    def test_falls_back_to_go_file(self):
        us_dir = os.path.join(tempfile.mkdtemp(), 'nonexistent')
        go_dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, go_dir, True)
        go_file = os.path.join(go_dir, 'go')
        dest = mod.write_boot_persistence(
            ['mutagen'], '/mnt/vms/x/tools/ImportLidarrManual.py',
            user_scripts_dir=us_dir, go_file=go_file)
        self.assertEqual(dest, go_file)
        with open(go_file) as f:
            content = f.read()
        self.assertIn('pip install mutagen', content)
        self.assertIn('# >>> ImportLidarrManual boot setup >>>', content)


class TestEnsureIlSymlink(unittest.TestCase):
    def test_creates_link(self):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        script = os.path.join(d, 'ImportLidarrManual.py')
        open(script, 'w').close()
        link = os.path.join(d, 'il')
        self.assertTrue(mod.ensure_il_symlink(script, link_path=link))
        self.assertTrue(os.path.islink(link))
        self.assertEqual(os.path.realpath(link), os.path.realpath(script))

    def test_relinks_when_target_wrong(self):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        script = os.path.join(d, 'ImportLidarrManual.py')
        open(script, 'w').close()
        old = os.path.join(d, 'old')
        open(old, 'w').close()
        link = os.path.join(d, 'il')
        os.symlink(old, link)
        self.assertTrue(mod.ensure_il_symlink(script, link_path=link))
        self.assertEqual(os.path.realpath(link), os.path.realpath(script))


class TestPreflightDriver(unittest.TestCase):
    def test_noninteractive_reports_without_installing(self):
        # Force a missing dep and non-interactive mode; ensure no install runs.
        deps = [mod.Dependency('rsgain', 'binary', lambda: False, 'ReplayGain',
                               {'brew': 'rsgain', 'un-get': 'rsgain'},
                               needed_when=lambda a, s: not a.no_rsgain)]
        args = Namespace(skip_bpm=False, no_rsgain=False, no_preflight=False)

        orig_build = mod.build_dependencies
        installed = []
        orig_install = mod.install_dependency
        mod.build_dependencies = lambda: deps
        mod.install_dependency = lambda d, p: installed.append(d.name)
        try:
            mod.preflight_dependencies(args, [], interactive=False)  # must not raise, must not install
        finally:
            mod.build_dependencies = orig_build
            mod.install_dependency = orig_install
        self.assertEqual(installed, [])

    def test_no_preflight_flag_short_circuits(self):
        args = Namespace(skip_bpm=False, no_rsgain=False, no_preflight=True)
        called = []
        orig = mod.build_dependencies
        mod.build_dependencies = lambda: called.append(1) or []
        try:
            mod.preflight_dependencies(args, [])
        finally:
            mod.build_dependencies = orig
        self.assertEqual(called, [])


class TestScriptDir(unittest.TestCase):
    def test_resolves_through_symlink(self):
        # Real file and symlink live in SEPARATE dirs, so a revert to abspath
        # (which would return the symlink's dir) fails this test.
        real_dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, real_dir, ignore_errors=True)
        real_file = os.path.join(real_dir, 'ImportLidarrManual.py')
        open(real_file, 'w').close()
        link_dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, link_dir, ignore_errors=True)
        link = os.path.join(link_dir, 'il')
        os.symlink(real_file, link)
        # Invoked via the symlink, _script_dir must point at the REAL file's dir,
        # not the symlink's dir.
        self.assertEqual(mod._script_dir(link), os.path.realpath(real_dir))
        self.assertNotEqual(mod._script_dir(link), os.path.realpath(link_dir))

    def test_direct_path(self):
        real_dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, real_dir, ignore_errors=True)
        real_file = os.path.join(real_dir, 'ImportLidarrManual.py')
        open(real_file, 'w').close()
        self.assertEqual(mod._script_dir(real_file), os.path.realpath(real_dir))


class TestVerifySha256(unittest.TestCase):
    # sha256(b'hello world')
    HELLO = 'b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9'

    def test_match(self):
        self.assertTrue(mod._verify_sha256(b'hello world', self.HELLO))

    def test_match_is_case_insensitive(self):
        self.assertTrue(mod._verify_sha256(b'hello world', self.HELLO.upper()))

    def test_mismatch(self):
        self.assertFalse(mod._verify_sha256(b'hello world', '00' * 32))

    def test_mismatch_on_tampered_bytes(self):
        self.assertFalse(mod._verify_sha256(b'hello world!', self.HELLO))


class TestExtractMember(unittest.TestCase):
    def test_zip_member(self):
        buf = io.BytesIO()
        with zipfile.ZipFile(buf, 'w') as zf:
            zf.writestr('pkg/rsgain', b'BINARY')
        self.assertEqual(
            mod._extract_member(buf.getvalue(), 'x.zip', 'pkg/rsgain'), b'BINARY')

    def test_tar_xz_member(self):
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode='w:xz') as tf:
            data = b'BINARY'
            info = tarfile.TarInfo('pkg/rsgain')
            info.size = len(data)
            tf.addfile(info, io.BytesIO(data))
        self.assertEqual(
            mod._extract_member(buf.getvalue(), 'x.tar.xz', 'pkg/rsgain'), b'BINARY')

    def test_unknown_archive_type_returns_none(self):
        self.assertIsNone(mod._extract_member(b'whatever', 'x.bin', 'member'))


class TestResolveRsgainPrecedence(unittest.TestCase):
    def setUp(self):
        self._orig_bundled = mod.BUNDLED_RSGAIN
        self._orig_rsgain_bin = mod.RSGAIN_BIN
        self._orig_which = mod.shutil.which
        self.d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.d, True)

        def restore():
            mod.BUNDLED_RSGAIN = self._orig_bundled
            mod.RSGAIN_BIN = self._orig_rsgain_bin
            mod.shutil.which = self._orig_which
        self.addCleanup(restore)

    def _make_exe(self, name):
        path = os.path.join(self.d, name)
        open(path, 'w').close()
        os.chmod(path, 0o755)
        return path

    def test_bundled_wins(self):
        bundled = self._make_exe('bundled')
        rsgain_bin = self._make_exe('rsgain_bin')
        mod.BUNDLED_RSGAIN = bundled
        mod.RSGAIN_BIN = rsgain_bin
        mod.shutil.which = lambda _n: '/usr/bin/rsgain'
        self.assertEqual(mod.resolve_rsgain(), bundled)

    def test_rsgain_bin_when_no_bundled(self):
        rsgain_bin = self._make_exe('rsgain_bin')
        mod.BUNDLED_RSGAIN = os.path.join(self.d, 'absent')
        mod.RSGAIN_BIN = rsgain_bin
        mod.shutil.which = lambda _n: '/usr/bin/rsgain'
        self.assertEqual(mod.resolve_rsgain(), rsgain_bin)

    def test_path_fallback(self):
        mod.BUNDLED_RSGAIN = os.path.join(self.d, 'absent1')
        mod.RSGAIN_BIN = os.path.join(self.d, 'absent2')
        mod.shutil.which = lambda _n: '/usr/bin/rsgain'
        self.assertEqual(mod.resolve_rsgain(), '/usr/bin/rsgain')

    def test_non_executable_bundled_is_skipped(self):
        bundled = os.path.join(self.d, 'bundled')
        open(bundled, 'w').close()
        os.chmod(bundled, 0o644)  # not executable
        rsgain_bin = self._make_exe('rsgain_bin')
        mod.BUNDLED_RSGAIN = bundled
        mod.RSGAIN_BIN = rsgain_bin
        mod.shutil.which = lambda _n: None
        self.assertEqual(mod.resolve_rsgain(), rsgain_bin)


class TestGetConfigDir(unittest.TestCase):
    def test_unraid_is_next_to_script(self):
        self.assertEqual(mod.get_config_dir('unraid'), mod.SCRIPT_DIR)

    def test_linux_uses_xdg(self):
        orig = os.environ.get('XDG_CONFIG_HOME')
        os.environ['XDG_CONFIG_HOME'] = '/tmp/xdghome'
        self.addCleanup(
            lambda: os.environ.__setitem__('XDG_CONFIG_HOME', orig)
            if orig is not None else os.environ.pop('XDG_CONFIG_HOME', None))
        self.assertEqual(mod.get_config_dir('linux'),
                         '/tmp/xdghome/importlidarr')


class TestEnvDiscoveryPaths(unittest.TestCase):
    def test_env_file_takes_precedence(self):
        paths = mod.env_discovery_paths(Namespace(env_file='/custom/.env'))
        self.assertEqual(paths[0], '/custom/.env')
        # XDG, next-to-script, and CWD follow.
        self.assertEqual(len(paths), 4)
        self.assertEqual(paths[-1], os.path.join(os.getcwd(), '.env'))

    def test_without_env_file_xdg_is_first(self):
        orig = os.environ.get('XDG_CONFIG_HOME')
        os.environ['XDG_CONFIG_HOME'] = '/tmp/xdghome'
        self.addCleanup(
            lambda: os.environ.__setitem__('XDG_CONFIG_HOME', orig)
            if orig is not None else os.environ.pop('XDG_CONFIG_HOME', None))
        paths = mod.env_discovery_paths(Namespace(env_file=None))
        self.assertEqual(len(paths), 3)
        self.assertEqual(paths[0], '/tmp/xdghome/importlidarr/.env')
        self.assertEqual(paths[-1], os.path.join(os.getcwd(), '.env'))


class TestScaffoldEnvFile(unittest.TestCase):
    def _args(self, **kw):
        base = dict(url=None, api_key=None, local_path=None, remote_path=None)
        base.update(kw)
        return Namespace(**base)

    def test_noninteractive_writes_0600(self):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        env_path = os.path.join(d, '.env')
        args = self._args(url='http://lidarr:8686', api_key='SECRET',
                          local_path='/mnt/user', remote_path='/share')
        self.assertTrue(mod.scaffold_env_file(env_path, args, interactive=False))
        self.assertEqual(os.stat(env_path).st_mode & 0o777, 0o600)
        with open(env_path) as f:
            body = f.read()
        self.assertIn('LIDARR_URL=http://lidarr:8686', body)
        self.assertIn('LIDARR_API_KEY=SECRET', body)
        self.assertIn('LIDARR_LOCAL_PATH=/mnt/user', body)
        self.assertIn('LIDARR_REMOTE_PATH=/share', body)

    def test_interactive_overwrite_retightens_perms(self):
        # A pre-existing world-readable .env must end up 0600 after an
        # accepted interactive overwrite (the security contract).
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        env_path = os.path.join(d, '.env')
        with open(env_path, 'w') as f:
            f.write('LIDARR_URL=old\n')
        os.chmod(env_path, 0o644)
        # input() is a builtin; confirm the overwrite, skip optional prompts.
        import builtins
        orig = builtins.input
        builtins.input = lambda prompt='': 'y' if 'overwrite' in prompt.lower() else ''
        self.addCleanup(setattr, builtins, 'input', orig)
        args = self._args(url='http://new:8686', api_key='NEW')
        self.assertTrue(mod.scaffold_env_file(env_path, args, interactive=True))
        self.assertEqual(os.stat(env_path).st_mode & 0o777, 0o600)
        with open(env_path) as f:
            self.assertIn('http://new:8686', f.read())

    def test_noninteractive_missing_values_skips(self):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        env_path = os.path.join(d, '.env')
        # Ensure env vars don't accidentally satisfy it.
        for var in ('LIDARR_URL', 'LIDARR_API_KEY'):
            orig = os.environ.pop(var, None)
            if orig is not None:
                self.addCleanup(os.environ.__setitem__, var, orig)
        args = self._args()
        self.assertFalse(mod.scaffold_env_file(env_path, args, interactive=False))
        self.assertFalse(os.path.exists(env_path))

    def test_no_clobber_when_noninteractive(self):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        env_path = os.path.join(d, '.env')
        with open(env_path, 'w') as f:
            f.write('LIDARR_URL=existing\n')
        args = self._args(url='http://new:8686', api_key='NEW')
        self.assertFalse(mod.scaffold_env_file(env_path, args, interactive=False))
        with open(env_path) as f:
            self.assertIn('existing', f.read())


class TestSetupPipPackages(unittest.TestCase):
    def test_includes_optional_cv2(self):
        pkgs = mod.setup_pip_packages(mod.build_dependencies())
        # cv2 is optional but --setup installs it, so it MUST also be in the
        # boot-persist list or it would silently vanish on an Unraid reboot.
        self.assertIn('opencv-python-headless', pkgs)

    def test_includes_non_optional_pip_deps(self):
        pkgs = mod.setup_pip_packages(mod.build_dependencies())
        self.assertIn('mutagen', pkgs)
        self.assertIn('essentia', pkgs)

    def test_only_pip_kind_and_sorted(self):
        pkgs = mod.setup_pip_packages(mod.build_dependencies())
        # binaries (rsgain, ffmpeg, magick, git) must not appear
        self.assertNotIn('rsgain', pkgs)
        self.assertNotIn('git', pkgs)
        self.assertEqual(pkgs, sorted(pkgs))


class TestRedactUrl(unittest.TestCase):
    def test_strips_userinfo(self):
        self.assertEqual(
            mod.redact_url('http://user:pass@host:8096/api'),
            'http://host:8096/api')

    def test_strips_username_only(self):
        self.assertEqual(
            mod.redact_url('http://user@host:8096'), 'http://host:8096')

    def test_passthrough_clean_url(self):
        for url in ('http://localhost:8686', 'https://lidarr.example.com/x?y=1'):
            self.assertEqual(mod.redact_url(url), url)

    def test_no_credentials_leak_in_output(self):
        self.assertNotIn('secret', mod.redact_url('http://u:secret@h:8096'))

    def test_handles_garbage_without_raising(self):
        # never raises; returns something for non-URL input
        self.assertIsInstance(mod.redact_url('not a url'), str)
        self.assertIsInstance(mod.redact_url(''), str)


if __name__ == '__main__':
    unittest.main()
