#!/bin/sh
set -e

# If running as root, ensure /app/data is owned by the app user and drop privileges
if [ "$(id -u)" = '0' ]; then
    mkdir -p /app/data
    chown -R app:app /app/data
    exec gosu app "$@"
fi

exec "$@"
