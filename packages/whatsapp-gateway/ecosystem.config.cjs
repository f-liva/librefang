module.exports = {
  apps: [{
    name: 'whatsapp-gateway',
    script: 'index.js',
    cwd: '/data/whatsapp-gateway',
    watch: false,
    autorestart: true,
    max_restarts: 5,
    min_uptime: '30s',
    restart_delay: 3000,
    max_memory_restart: '256M',
    exp_backoff_restart_delay: 1000,
    error_file: '/data/whatsapp-gateway/logs/pm2-error.log',
    out_file: '/data/whatsapp-gateway/logs/pm2-out.log',
    merge_logs: true,
    time: true,
    env: {
      NODE_ENV: 'production',
      OPENFANG_DEFAULT_AGENT: 'ambrogio',
    },
  }],
};
