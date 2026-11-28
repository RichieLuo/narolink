#!/usr/bin/env python3
"""Install a verified Naro bundle; restart only after all hosted tasks become idle.

Usage: python3 -I desktop/scripts/install-macos.py STAGING_DIRECTORY
Staging must contain Naro.app, web.sha256, version and open-naro (NSWorkspace launcher).
"""
from hashlib import sha256
import hmac
import json
import os
from pathlib import Path
import plistlib
import subprocess
import sys
import time
from urllib.request import Request, urlopen

STAGE = Path(sys.argv[1]).resolve()
APP = Path('/Applications/Naro.app')
SOURCE = STAGE / 'Naro.app'
STATE = Path.home() / 'Library/Application Support/narolink'
LOCAL = 'http://127.0.0.1:4318'
PUBLIC = 'https://naro.bato.chat'
VERSION = (STAGE / 'version').read_text().strip()
STATUS = STAGE / 'installation-status.json'
BINARY = Path('Contents/MacOS/narolink')


def report(phase, **extra):
    data = {'phase': phase, 'version': VERSION, 'time': time.strftime('%Y-%m-%dT%H:%M:%S%z'), **extra}
    temp = STATUS.with_suffix('.tmp')
    temp.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n')
    os.replace(temp, STATUS)
    print(json.dumps(data, ensure_ascii=False), flush=True)


def request(origin, route, cookie='', body=None):
    headers = {'Cookie': cookie} if cookie else {}
    if body is not None:
        headers.update({'Origin': origin, 'Content-Type': 'application/json'})
    with urlopen(Request(origin + route, headers=headers, data=json.dumps(body).encode() if body is not None else None), timeout=12) as response:
        data = response.read()
        return (json.loads(data) if data else None), response.headers.get('Set-Cookie')


def pair():
    key = (STATE / 'pairing.key').read_bytes()
    tag = hmac.digest(key, f'pair:{int(time.time()) // 300}'.encode(), 'sha256')
    code = str(int.from_bytes(tag[:8], 'big') % 100000000).zfill(8)
    return request(LOCAL, '/api/pair', body={'code': code})[1].split(';')[0]


def verify_assets(origin):
    for line in (STAGE / 'web.sha256').read_text().splitlines():
        digest, asset = line.split(maxsplit=1)
        route = '' if asset == 'index.html' else asset
        with urlopen(origin + '/' + route, timeout=15) as response:
            assert sha256(response.read()).hexdigest() == digest, 'Asset mismatch: ' + asset


def verify_binary():
    assert sha256((APP / BINARY).read_bytes()).digest() == sha256((SOURCE / BINARY).read_bytes()).digest()
    subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', APP], check=True)


def restart():
    key_digest = sha256((STATE / 'pairing.key').read_bytes()).digest()
    cookie = pair()
    stable = None
    report('installed_waiting_for_idle')
    while True:
        try:
            running, _ = request(LOCAL, '/api/runs', cookie)
            if running:
                stable = None
            elif stable is None:
                stable = time.monotonic()
            elif time.monotonic() - stable >= 8 and not request(LOCAL, '/api/runs', cookie)[0]:
                break
        except Exception:
            stable = None
            cookie = pair()
        time.sleep(2)
    verify_binary()
    report('restarting_after_idle')
    subprocess.run(['/usr/bin/osascript', '-e', 'tell application id "chat.bato.narolink" to quit'], check=True, timeout=20)
    for _ in range(80):
        if subprocess.run(['/usr/bin/pgrep', '-f', '^/Applications/Naro\\.app/Contents/MacOS/narolink$'], capture_output=True).returncode:
            break
        time.sleep(.25)
    else:
        raise RuntimeError('Naro did not quit normally; no forced termination attempted')
    token = (Path.home() / '.config/narolink/relay-token').read_text().strip()
    subprocess.run([STAGE / 'open-naro'], input=token + '\n', text=True, check=True, timeout=30)
    last = None
    for _ in range(60):
        try:
            verify_binary()
            verify_assets(LOCAL)
            cookie = pair()
            assert request(LOCAL, '/api/access', cookie)[0]['paired']
            key = (STATE / 'pairing.key').read_bytes()
            assert sha256(key).digest() == key_digest
            device = sha256(hmac.digest(key, b'relay-device-key:v1', 'sha256')).hexdigest()[:32]
            assert request(PUBLIC, '/devices/' + device + '/api/access')[0]['online']
            report('restarted_verified', pairing_preserved=True, local_assets_match=True, remote_online=True)
            return
        except Exception as error:
            last = str(error)
            time.sleep(2)
    raise RuntimeError('Post-restart verification failed: ' + str(last))


def install():
    assert APP.is_dir() and SOURCE.is_dir()
    assert all(c.isalnum() or c in '._-' for c in VERSION)
    verify_assets(PUBLIC)
    subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', SOURCE], check=True)
    backup = STATE / 'backups' / ('Naro-before-' + VERSION + '.app')
    staged = APP.parent / ('.Naro.app.' + VERSION)
    assert not backup.exists() and not staged.exists()
    backup.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(['/usr/bin/ditto', SOURCE, staged], check=True)
    os.rename(APP, backup)
    try:
        os.rename(staged, APP)
        verify_binary()
        report('installed_on_disk', backup=str(backup))
    except Exception:
        if APP.exists():
            os.rename(APP, staged)
        os.rename(backup, APP)
        raise
    # launchd cannot read a removable-volume workspace under macOS privacy rules.
    # Stage the already-inspected updater outside that volume before scheduling.
    updater = STAGE / 'install-macos.py'
    if Path(__file__).resolve() != updater:
        updater.write_bytes(Path(__file__).read_bytes())
    label = 'chat.bato.narolink.upgrade.' + VERSION
    plist = STAGE / 'restart-once.plist'
    plist.write_bytes(plistlib.dumps({'Label': label, 'ProgramArguments': ['/usr/bin/python3', '-I', str(updater), str(STAGE), 'restart'], 'RunAtLoad': True, 'KeepAlive': False, 'StandardOutPath': str(STAGE / 'restart.log'), 'StandardErrorPath': str(STAGE / 'restart-error.log')}))
    subprocess.run(['/bin/launchctl', 'bootstrap', 'gui/' + str(os.getuid()), plist], check=True)
    report('installed_restart_scheduled', backup=str(backup))


try:
    restart() if len(sys.argv) > 2 and sys.argv[2] == 'restart' else install()
except Exception as error:
    report('failed', error=str(error))
    raise
