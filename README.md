# OpenAI Token-Auth Reverse Proxy (Rust)

Лёгкий reverse-proxy на Rust для безопасного доступа к OpenAI из ваших сервисов.

## Возможности
- Авторизация клиентов через `Authorization: Bearer <SERVICE_TOKEN>`
- Проксирование любых путей под `/v1/**` на `https://api.openai.com/v1/**`
- Подстановка `Authorization: Bearer <OPENAI_API_KEY>` к OpenAI
- Стриминг ответов (поддержка SSE/`stream: true`)
- Health-check: `/healthz`
- Gzip/br сжатие и HTTP-трейс-логи

## Запуск

```bash
cp .env.example .env
# заполните SERVICE_TOKEN и OPENAI_API_KEY

cargo run --release
# слушает по умолчанию :8080

Проверка:

curl -s http://127.0.0.1:8080/healthz
# ok

 OpenAI — прокси сам подставит его

Пример (Chat Completions):

curl -N http://127.0.0.1:8080/v1/chat/completions \
  -H "Authorization: Bearer REPLACE_WITH_SERVICE_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "model":"gpt-4o-mini",
    "messages":[{"role":"user","content":"Hello!"}],
    "stream": true
  }'

Деплой

Systemd, Docker, Kubernetes — без ограничений (это обычный HTTP-сервис).

Убедитесь, что прокси доступен только доверенным клиентам (VPC, mTLS, allow-list), поскольку один сервисный токен общий для всех клиентов.

Примечания безопасности

Храните OPENAI_API_KEY только в окружении прокси.

Регулярно ротируйте SERVICE_TOKEN.

При необходимости добавьте rate-limit, audit-лог и IP-allowlist (легко расширяется в axum).

