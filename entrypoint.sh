#!/bin/bash
# Drop root privileges and run librefang as the librefang user
chown -R librefang:librefang /data 2>/dev/null
chown -R librefang:librefang /home/librefang 2>/dev/null

# Resurrect PM2 processes (whatsapp-gateway etc.) before starting LibreFang
gosu librefang bash -c 'pm2 resurrect 2>/dev/null || true'

# Restore MemPalace setup from persistent /data/ volume
if [ -d /data/mempalace ]; then
  gosu librefang bash -c '
    # Config pointer
    mkdir -p ~/.mempalace
    cp -n /data/mempalace/config.json ~/.mempalace/config.json 2>/dev/null || true

    # Plugin symlink
    mkdir -p ~/.librefang/plugins
    ln -sf /data/mempalace/plugin ~/.librefang/plugins/mempalace-indexer 2>/dev/null || true

    # Rebuild venv if missing (idempotent, cached wheels make this fast)
    if [ ! -f ~/.mempalace-venv/bin/python ]; then
      ~/.local/bin/uv venv ~/.mempalace-venv --python python3 2>/dev/null
      ~/.local/bin/uv pip install mempalace --python ~/.mempalace-venv/bin/python 2>/dev/null
    fi
  '
fi

# Load secrets.env into the environment before launching the kernel.
# Upstream PR #2359 makes the kernel load this file autonomously, but we
# source it here too so older images keep working and so the env is
# already populated when `gosu` execs into the librefang user.
if [ -f /data/secrets.env ]; then
  set -a
  # shellcheck disable=SC1091
  source /data/secrets.env
  set +a
fi

exec gosu librefang librefang start --foreground
