#!/bin/bash
# Drop root privileges and run librefang as the librefang user
chown -R librefang:librefang /data 2>/dev/null
chown -R librefang:librefang /home/librefang 2>/dev/null

# Resurrect PM2 processes (whatsapp-gateway etc.) before starting LibreFang
gosu librefang bash -c 'pm2 resurrect 2>/dev/null || true'

exec gosu librefang librefang start --foreground
