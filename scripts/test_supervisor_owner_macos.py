#!/usr/bin/env python3
"""Exercise the real LaunchAgent in an isolated home before installing it for a user.

Requires built debug binaries and a macOS GUI login. Refuses to run if the RZN
LaunchAgent is already loaded. Only the fixture's copied executables are signaled.
"""
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time


LABEL = "com.rzn.browser.runtime-owner"
REPO = Path(__file__).resolve().parents[1]


def main():
    if sys.platform != "darwin":
        raise SystemExit("This check requires macOS and a GUI login.")
    job = f"gui/{os.getuid()}/{LABEL}"
    loaded = subprocess.run(["launchctl", "print", job], capture_output=True)
    if loaded.returncode == 0:
        raise SystemExit("Refusing to touch an existing RZN runtime owner. Use a clean test login.")
    sources = {name: REPO / "target/debug" / name for name in ("rzn-browser", "rzn-native-host")}
    for source in sources.values():
        if not source.is_file():
            raise SystemExit(f"Missing {source}; run make build-rust first.")

    root = Path(tempfile.mkdtemp(prefix="rzn-owner-chaos-", dir="/tmp")).resolve()
    home, app = root / "home", root / "app"
    home.mkdir()
    (app / "bin").mkdir(parents=True)
    real_cli = app / "bin/rzn-browser-real"
    native = app / "bin/rzn-native-host"
    shutil.copy2(sources["rzn-browser"], real_cli)
    shutil.copy2(sources["rzn-native-host"], native)
    env = {**os.environ, "HOME": str(home), "RZN_APP_BASE_DIR": str(app)}
    unset_keys = (
        "RZN_CLOUD_CONFIG_PATH", "RZN_LOCAL_RUNTIME_SOCKET_PATH", "RZN_SUPERVISOR_SOCKET_PATH",
        "RZN_LOCAL_RUNTIME_TOKEN_PATH", "RZN_SUPERVISOR_TOKEN_PATH",
    )
    for key in unset_keys:
        env.pop(key, None)
    cli = app / "bin/rzn-browser"
    # launchd inherits the login environment, so the fixture's executable wrapper
    # must also isolate cloud config/journals. Production uses the binary directly.
    cli.write_text(
        "#!/bin/sh\n"
        f"export HOME={shlex.quote(str(home))}\n"
        f"export RZN_APP_BASE_DIR={shlex.quote(str(app))}\n"
        f"unset {' '.join(unset_keys)}\n"
        f"exec {shlex.quote(str(real_cli))} \"$@\"\n"
    )
    cli.chmod(0o700)
    owner_file = app / "run/rzn-supervisor-owner.json"
    native_process = None
    installed = False
    success = False

    def command(*args, timeout=80, check=True):
        result = subprocess.run(
            [str(cli), "supervisor", *args, "--app-base", str(app), "--json"],
            env=env, capture_output=True, text=True, timeout=timeout,
        )
        if check and result.returncode:
            raise AssertionError(f"{' '.join(args)} failed: {result.stderr}\n{result.stdout}")
        return result

    def call(method, params=None, **kwargs):
        result = command("call", method, "--params", json.dumps(params or {}), **kwargs)
        return json.loads(result.stdout)

    def status():
        try:
            return json.loads(command("status", timeout=4).stdout)
        except (AssertionError, subprocess.TimeoutExpired, json.JSONDecodeError):
            return {}

    def owner():
        try:
            return json.loads(owner_file.read_text())
        except (FileNotFoundError, json.JSONDecodeError):
            return {}

    def wait_for(predicate, description, timeout=75):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                return value
            time.sleep(0.25)
        raise AssertionError(f"Timed out: {description}; status={status()}; owner={owner()}")

    def ready(exclude=None):
        result = status()
        pid = result.get("pid")
        return pid if pid and pid != exclude and owner().get("child_pid") == pid else None

    def signal_owned(pid, sig):
        result = subprocess.run(["ps", "-p", str(pid), "-o", "command="], capture_output=True, text=True)
        if str(real_cli) not in result.stdout:
            raise AssertionError(f"Refusing to signal a process outside the fixture: {pid}")
        os.kill(pid, sig)

    def passed(name):
        print(f"PASS {name}", flush=True)

    try:
        # Warm copied Mach-O executables before timing launchd or browser IPC.
        subprocess.run([str(real_cli), "--help"], env=env, capture_output=True, check=True, timeout=30)
        subprocess.run([str(native), "--self-test"], env=env, capture_output=True, check=True, timeout=30)
        installed = True  # cleanup also covers a partially completed bootstrap
        command("service", "install")
        child = wait_for(ready, "initial service startup")
        initial_owner = owner()["pid"]
        command("service", "install")
        assert ready() == child and owner()["pid"] == initial_owner, "idempotent install restarted runtime"
        passed("install and idempotent install")

        signal_owned(child, signal.SIGKILL)
        child = wait_for(lambda: ready(child), "child crash recovery")
        passed("killed supervisor restarts without a client request")

        signal_owned(child, signal.SIGSTOP)
        child = wait_for(lambda: ready(child), "stopped child watchdog recovery")
        passed("unresponsive supervisor is replaced by its owner")

        previous_owner = owner()["pid"]
        signal_owned(previous_owner, signal.SIGKILL)
        wait_for(lambda: owner().get("pid") not in (None, previous_owner), "launchd owner crash recovery")
        child = wait_for(lambda: ready(child), "new owner's child startup")
        passed("launchd restarts a killed owner and replaces its child")

        command("shutdown")
        wait_for(lambda: not status().get("pid"), "managed shutdown", timeout=15)
        native_process = subprocess.Popen(
            [str(native), "chrome-extension://bogjdnehdficgkhklinmnbgiiofbamji/"],
            env=env, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        child = wait_for(lambda: ready(child), "native-host demand resumes parked owner")
        native_process.terminate()
        native_process.wait(timeout=10)
        native_process = None
        passed("native-host demand resumes a deliberately stopped supervisor")

        call("runs.claim", {"run_id": "owner-fixture", "workflow_id": "fixture", "origin": "local_cli", "owner_pid": os.getpid()})
        blocked = command("service", "install", "--restart", check=False)
        assert blocked.returncode != 0, "upgrade interrupted an active workflow"
        assert ready() == child, "active workflow refusal replaced supervisor"
        call("runs.release", {"run_id": "owner-fixture"})
        signal_owned(child, signal.SIGKILL)
        child = wait_for(lambda: ready(child), "recovery after refused upgrade")
        passed("upgrade refuses active workflow and leaves automatic recovery enabled")

        command("service", "install", "--restart")
        child = wait_for(lambda: ready(child), "explicit service upgrade")
        passed("explicit upgrade replaces the runtime")

        command("service", "uninstall")
        installed = False
        assert subprocess.run(["launchctl", "print", job], capture_output=True).returncode != 0
        assert not (app / "run/supervisor-owner.json").exists()
        wait_for(lambda: not status().get("pid"), "uninstall shutdown", timeout=15)
        passed("uninstall removes service and managed marker")
        success = True
    finally:
        if native_process is not None:
            native_process.terminate()
            try:
                native_process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                native_process.kill()
                native_process.wait(timeout=10)
        if installed:
            current_job = subprocess.run(["launchctl", "print", job], capture_output=True, text=True)
            if current_job.returncode == 0 and str(cli) in current_job.stdout:
                subprocess.run(["launchctl", "bootout", job], capture_output=True, timeout=30)
            elif current_job.returncode == 0:
                print("Refusing cleanup of a launchd job outside the fixture", file=sys.stderr)
        if success:
            shutil.rmtree(root)
        else:
            print(f"Fixture retained for diagnostics: {root}", file=sys.stderr)


if __name__ == "__main__":
    main()
