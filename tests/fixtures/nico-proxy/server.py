# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
"""Loopback-only mTLS proxy fixture; all identities are generated at test time."""
import http.server
import json
from pathlib import Path
import ssl
import subprocess
import sys

root = Path(sys.argv[1])

def openssl(*arguments):
    subprocess.run(["openssl", *arguments], cwd=root, check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

openssl("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
        "-subj", "/CN=Fixture CA", "-keyout", "ca.key", "-out", "ca.crt")
for name, extension in [("server", "subjectAltName=IP:127.0.0.1\nextendedKeyUsage=serverAuth"),
                        ("client", "extendedKeyUsage=clientAuth")]:
    openssl("req", "-new", "-newkey", "rsa:2048", "-nodes", "-subj", f"/CN={name}",
            "-keyout", f"{name}.key", "-out", f"{name}.csr")
    (root / f"{name}.ext").write_text(extension + "\n")
    openssl("x509", "-req", "-in", f"{name}.csr", "-CA", "ca.crt", "-CAkey", "ca.key",
            "-CAcreateserial", "-days", "1", "-extfile", f"{name}.ext", "-out", f"{name}.crt")

component = "/redfish/v1/ComponentIntegrity/GPU_0_IRoT"
action = component + "/Actions/ComponentIntegrity.SPDMGetSignedMeasurements"

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def respond(self, status, body):
        payload = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def handle_request(self):
        if (self.headers.get("Forwarded") != 'host="192.0.2.1"'
                or any(self.headers.get(name) for name in
                       ("Authorization", "Proxy-Authorization", "X-Auth-Token"))
                or not self.connection.getpeercert()):
            self.respond(403, {})
            return
        with (root / "requests").open("a") as capture:
            capture.write(f"{self.command} {self.path}\n")
        if self.command == "POST":
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            if self.path != action or body != {"SlotId": 0, "MeasurementIndices": [255], "Nonce": "backend-issued-test-nonce"}:
                self.respond(400, {})
                return
            self.respond(200, {"@odata.id": "https://192.0.2.1/redfish/v1/TaskService/Tasks/1"})
            return
        routes = {
            "/redfish/v1": {"@odata.id": "/redfish/v1", "@odata.type": "#ServiceRoot.v1_15_0.ServiceRoot",
                            "Id": "RootService", "Name": "Fixture", "RedfishVersion": "1.15.1",
                            "Links": {"Sessions": {"@odata.id": "/redfish/v1/SessionService/Sessions"}},
                            "Systems": {"@odata.id": "/redfish/v1/Systems"}},
            "/redfish/v1/Systems": {"@odata.id": "/redfish/v1/Systems", "@odata.type": "#ComputerSystemCollection.ComputerSystemCollection",
                                    "Name": "Systems", "Members": [], "Members@odata.count": 0},
            "/redfish/v1/ComponentIntegrity": {"Members": [{"@odata.id": component}]},
            component: {"Id": "GPU_0_IRoT", "Actions": {"#ComponentIntegrity.SPDMGetSignedMeasurements": {"target": "https://192.0.2.1" + action}}},
            "/redfish/v1/TaskService/Tasks/1": {"TaskState": "Completed"},
            action + "/data": {"SignedMeasurements": "signed-test-evidence", "Version": "1"},
            "/redfish/v1/Chassis/GPU_0_IRoT/Certificates/CertChain": {"CertificateString": "test-certificate-chain"},
            "/redfish/v1/Chassis/GPU_0_IRoT": {"Model": "B200"},
        }
        self.respond(200 if self.path in routes else 404, routes.get(self.path, {}))

    do_GET = handle_request
    do_POST = handle_request

server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(root / "server.crt", root / "server.key")
context.load_verify_locations(root / "ca.crt")
context.verify_mode = ssl.CERT_REQUIRED
server.socket = context.wrap_socket(server.socket, server_side=True)
print(server.server_port, flush=True)
server.serve_forever()
