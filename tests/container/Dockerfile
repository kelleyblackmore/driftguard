# Test target for the container-based SSH integration test.
# Simulates a freshly provisioned server: sshd for access, nginx as the
# deployed workload, and a templated app config to assert against.
FROM ubuntu:24.04

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates \
        iproute2 \
        nginx \
        openssh-server \
        procps \
    && rm -rf /var/lib/apt/lists/*

# Unprivileged user driftguard connects as; its authorized key is injected
# at runtime via the AUTHORIZED_KEY environment variable (see entrypoint).
RUN useradd -m -s /bin/bash driftguard \
    && mkdir -p /home/driftguard/.ssh /run/sshd \
    && chmod 700 /home/driftguard/.ssh \
    && chown -R driftguard:driftguard /home/driftguard/.ssh

# A rendered "application config" like Ansible/Terraform templates produce.
RUN mkdir -p /etc/testapp \
    && printf '{"service": {"enabled": true, "port": 8080, "environment": "production"}}\n' \
        > /etc/testapp/config.json

COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

EXPOSE 22 80
ENTRYPOINT ["/entrypoint.sh"]
