# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Local mTLS smoke tests; build bmc-collector before running this module."""

import http.server
import json
import os
from pathlib import Path
import shutil
import ssl
import subprocess
import tempfile
import threading
import unittest
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get("BMC_PROXY_TLS_BINARY", ROOT / "target/debug/bmc-collector"))


@unittest.skipUnless(shutil.which("openssl") and BINARY.exists(), "requires openssl and a built bmc-collector")
class BmcProxyTlsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="fleetint-proxy-tls-")
        cls.directory = Path(cls.temporary.name)

        def openssl(*arguments):
            subprocess.run(["openssl", *arguments], cwd=cls.directory,
                           check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

        for name in ("ca", "untrusted"):
            openssl("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                    "-subj", f"/CN={name}", "-keyout", f"{name}.key", "-out", f"{name}.crt")
        for name, extensions in (
            ("server", "extendedKeyUsage=serverAuth\nsubjectAltName=IP:127.0.0.1\n"),
            ("client", "extendedKeyUsage=clientAuth\n"),
        ):
            (cls.directory / f"{name}.ext").write_text("basicConstraints=CA:FALSE\n" + extensions)
            openssl("req", "-newkey", "rsa:2048", "-nodes", "-subj", f"/CN={name}",
                    "-keyout", f"{name}.key", "-out", f"{name}.csr")
            openssl("x509", "-req", "-in", f"{name}.csr", "-CA", "ca.crt", "-CAkey", "ca.key",
                    "-CAcreateserial", "-days", "1", "-extfile", f"{name}.ext", "-out", f"{name}.crt")
        for name in ("ca.key", "untrusted.key", "server.key", "client.key"):
            (cls.directory / name).chmod(0o600)
        (cls.directory / "oauth-id").write_text("fixture-client")
        (cls.directory / "oauth-secret").write_text("fixture-secret")

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def setUp(self):
        self.proxy_requests = []
        self.proxy_status = 200
        self.unexpected_api_requests = []
        test = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                self.rfile.read(int(self.headers.get("Content-Length", "0")))
                self.reply(200, {"access_token": "fixture-token", "token_type": "Bearer"})

            def do_GET(self):
                path = urlsplit(self.path).path
                if self.server.is_proxy:
                    test.proxy_requests.append((path, dict(self.headers), self.connection.getpeercert()))
                    self.reply(test.proxy_status, {
                        "@odata.id": "/redfish/v1", "@odata.type": "#ServiceRoot.v1_15_0.ServiceRoot",
                        "Id": "RootService", "Name": "Fixture", "RedfishVersion": "1.15.1",
                        "Links": {"Sessions": {"@odata.id": "/redfish/v1/SessionService/Sessions"}},
                    }, location=f"{test.api_origin}/unexpected-redirect")
                elif path.endswith("/site"):
                    self.reply(200, [{"id": "site-1", "name": "fixture-site"}], pagination=True)
                elif path.endswith("/machine"):
                    self.reply(200, [{"id": "machine-1", "siteId": "site-1", "hostname": "fixture-node",
                                      "metadata": {"bmcInfo": {"ip": "192.0.2.10", "mac": "00:11:22:33:44:55"}}}],
                               pagination=True)
                else:
                    test.unexpected_api_requests.append(path)
                    self.reply(500, {})

            def reply(self, status, payload, *, pagination=False, location=None):
                data = json.dumps(payload).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                if pagination:
                    self.send_header("X-Pagination", json.dumps({"pageNumber": 1, "pageSize": 100, "total": 1}))
                if location:
                    self.send_header("Location", location)
                self.end_headers()
                self.wfile.write(data)

        self.servers = []
        for is_proxy in (False, True):
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.load_cert_chain(self.directory / "server.crt", self.directory / "server.key")
            if is_proxy:
                context.verify_mode = ssl.CERT_REQUIRED
                context.load_verify_locations(self.directory / "ca.crt")
            server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
            server.is_proxy = is_proxy
            server.socket = context.wrap_socket(server.socket, server_side=True)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            self.servers.append((server, thread))
        self.api_origin = f"https://127.0.0.1:{self.servers[0][0].server_port}"
        self.proxy_origin = f"https://127.0.0.1:{self.servers[1][0].server_port}"

    def tearDown(self):
        for server, thread in self.servers:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)

    def collect(self, ca="ca.crt", key="client.key"):
        config = self.directory / "config.toml"
        config.write_text(f'''[endpoint_source]
        type = "nico"
        api_url = "{self.api_origin}/v2"
        org = "example"
        token_url = "{self.api_origin}/token"
        client_id_file = "oauth-id"
        client_secret_file = "oauth-secret"
        [bmc_proxy]
        url = "{self.proxy_origin}"
        ca_cert_path = "{ca}"
        client_cert_path = "client.crt"
        client_key_path = "{key}"
        ''')
        environment = {key: value for key, value in os.environ.items()
                       if key.upper() not in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY")}
        environment["SSL_CERT_FILE"] = str(self.directory / "ca.crt")
        return subprocess.run([str(BINARY), "inventory", "--config", str(config), "--format", "json",
                               "--disable-latency-probe", "--timeout-seconds", "2", "--insecure-tls"],
                              env=environment, capture_output=True, text=True, timeout=15)

    def test_mtls_inventory_without_bmc_passwords(self):
        result = self.collect()
        self.assertEqual(result.returncode, 0, result.stderr)
        inventory = json.loads(result.stdout)[0]
        self.assertIsNone(inventory.get("targetError"), result.stdout)
        self.assertEqual(inventory["source"]["mac"], "00:11:22:33:44:55")
        self.assertTrue(self.proxy_requests)
        for _, headers, certificate in self.proxy_requests:
            headers = {key.lower(): value for key, value in headers.items()}
            self.assertEqual(headers["forwarded"], 'mac="00:11:22:33:44:55"')
            self.assertNotIn("authorization", headers)
            self.assertNotIn("x-auth-token", headers)
            self.assertTrue(certificate)

    def test_untrusted_proxy_is_rejected_even_with_insecure_bmc_tls(self):
        result = self.collect(ca="untrusted.crt")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)[0].get("targetError"))
        self.assertEqual(self.proxy_requests, [])

    def test_missing_client_key_fails_before_redfish(self):
        result = self.collect(key="missing.key")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("failed to read bmc_proxy client key", result.stderr)
        self.assertEqual(self.proxy_requests, [])

    def test_proxy_redirect_is_not_followed(self):
        self.proxy_status = 307
        result = self.collect()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)[0].get("targetError"))
        self.assertEqual(self.unexpected_api_requests, [])
        self.assertEqual(len(self.proxy_requests), 1)


if __name__ == "__main__":
    unittest.main()
