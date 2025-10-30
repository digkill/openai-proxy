#!/usr/bin/env bash
set -euo pipefail

BIN_SRC="${1:-./openai-proxy}"

if [[ ! -f "$BIN_SRC" ]]; then
  echo "Usage: $0 /path/to/openai-proxy-binary"
  exit 1
fi

id -u openai-proxy >/dev/null 2>&1 || sudo useradd -r -s /usr/sbin/nologin openai-proxy

sudo mkdir -p /opt/openai-proxy
sudo cp "$BIN_SRC" /opt/openai-proxy/openai-proxy
sudo chown -R openai-proxy:openai-proxy /opt/openai-proxy
sudo chmod 0755 /opt/openai-proxy/openai-proxy

if [[ ! -f /etc/default/openai-proxy ]]; then
  sudo cp ./env/openai-proxy.env /etc/default/openai-proxy
  sudo chmod 0640 /etc/default/openai-proxy
  sudo chown root:openai-proxy /etc/default/openai-proxy
fi

sudo cp ./systemd/openai-proxy.service /etc/systemd/system/openai-proxy.service
sudo systemctl daemon-reload
sudo systemctl enable --now openai-proxy.service
sleep 1
sudo systemctl status --no-pager openai-proxy.service || true

if [[ -d /etc/nginx/sites-available ]]; then
  sudo cp ./nginx/openai-proxy.conf /etc/nginx/sites-available/openai-proxy.conf
  if [[ -d /etc/nginx/sites-enabled ]]; then
    sudo ln -sf /etc/nginx/sites-available/openai-proxy.conf /etc/nginx/sites-enabled/openai-proxy.conf
  fi
  echo "Remember to set server_name and enable TLS, then: sudo nginx -t && sudo systemctl reload nginx"
fi

echo "Done. Test: curl http://127.0.0.1:8080/healthz"
