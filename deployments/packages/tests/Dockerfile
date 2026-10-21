# SPDX-License-Identifier: Apache-2.0
FROM debian:12-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl openssl systemd socat util-linux && rm -rf /var/lib/apt/lists/*
RUN printf 'path-include=/usr/share/doc/fleetint-oob/*\n' > /etc/dpkg/dpkg.cfg.d/zz-fleetint-test
COPY deployments/packages/tests/smoke.sh /smoke.sh
COPY deployments/packages/tests/runtime.sh /runtime.sh
COPY deployments/packages/tests/fixtures/backend.sh /backend.sh
COPY deployments/packages/tests/lifecycle.sh /lifecycle.sh
RUN chmod 755 /backend.sh
CMD ["/bin/sh", "/smoke.sh"]
