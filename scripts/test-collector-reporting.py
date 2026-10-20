#!/usr/bin/env python3
"""Exercise the real collector against the local Postman reporting simulation."""

import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
BASE_URL = "http://127.0.0.1:4905/backend"


def main():
    chart = subprocess.check_output([
        "helm", "template", "smoke", str(ROOT / "deployments/helm/fleet-intelligence-oob-collector"),
        "--show-only", "templates/collector-deployment-configmap.yaml",
        "--set", "nicoHardwareHealth.enabled=false",
        "--set", "otelGateway.enabled=false",
        "--set", "nicoHardwareHealth.metricsPrefix=smoke",
    ], text=True)
    block = chart.split("  deployment.json: |\n", 1)[1]
    deployment = json.loads("\n".join(
        line[4:] for line in block.splitlines() if line.startswith("    ")
    ))
    with tempfile.TemporaryDirectory(prefix="collector-reporting-") as directory:
        work = Path(directory)
        target = work / "targets.toml"
        target.write_text('''[[targets]]
ip = "127.0.0.1"
port = 9
mac = "02:00:00:00:00:01"
username = "fixture-user"
password = "fixture-password"
''')
        snapshot = work / "deployment.json"
        snapshot.write_text(json.dumps(deployment))
        environment = {
            **os.environ, "FLEETINT_AGENT_TOKEN": "fixture-bootstrap-token",
            "RUST_LOG": "info", "NO_PROXY": "127.0.0.1,localhost",
        }
        with (work / "collector.log").open("w+") as log:
            process = subprocess.Popen([
                str(ROOT / "target/debug/bmc-collector"), "run",
                "--config", str(target), "--deployment-config", str(snapshot),
                "--backend-url", BASE_URL, "--interval-seconds", "3601",
                "--target-concurrency", "3", "--bmc-max-in-flight", "2",
                "--bmc-requests-per-second", "5", "--bmc-burst", "6",
                "--timeout-seconds", "2", "--disable-latency-probe",
            ], env=environment, stdout=log, stderr=log)
            try:
                for _ in range(100):
                    with urllib.request.urlopen(BASE_URL + "/__checks", timeout=2) as response:
                        received = json.load(response).get("config")
                    if received:
                        break
                    if process.poll() is not None:
                        raise RuntimeError("Collector exited before uploading")
                    time.sleep(0.1)
                else:
                    raise RuntimeError("No collector upload observed")
                assert received == {
                    "runtime": {
                        "inventoryIntervalSeconds": 3601, "targetConcurrency": 3,
                        "bmcMaxInFlight": 2, "bmcRequestsPerSecond": 5, "bmcBurst": 6,
                        "timeoutSeconds": 2, "insecureTls": False, "latencyProbe": False,
                        "logLevel": "info",
                    },
                    **deployment,
                }, received
                print("PASS: real collector uploaded resolved runtime and Helm configuration over HTTP")
            except Exception:
                log.seek(0)
                print(log.read())
                raise
            finally:
                if process.poll() is None:
                    process.send_signal(signal.SIGINT)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()


if __name__ == "__main__":
    main()
