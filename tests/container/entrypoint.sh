#!/bin/sh
set -e

if [ -n "$AUTHORIZED_KEY" ]; then
    echo "$AUTHORIZED_KEY" > /home/driftguard/.ssh/authorized_keys
    chown driftguard:driftguard /home/driftguard/.ssh/authorized_keys
    chmod 600 /home/driftguard/.ssh/authorized_keys
fi

nginx
exec /usr/sbin/sshd -D -e
