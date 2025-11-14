#!/bin/bash
# Генерация самоподписанного сертификата для локальной разработки

CERT_DIR="certs"
CERT_FILE="$CERT_DIR/localhost.crt"
KEY_FILE="$CERT_DIR/localhost.key"

# Создаем директорию для сертификатов
mkdir -p "$CERT_DIR"

# Генерируем самоподписанный сертификат
openssl req -x509 -newkey rsa:4096 -nodes \
    -keyout "$KEY_FILE" \
    -out "$CERT_FILE" \
    -days 365 \
    -subj "/C=RU/ST=State/L=City/O=Organization/CN=localhost" \
    -addext "subjectAltName=DNS:localhost,DNS:*.localhost,IP:127.0.0.1,IP:::1"

echo "Сертификат создан:"
echo "  Certificate: $CERT_FILE"
echo "  Private Key: $KEY_FILE"
echo ""
echo "Для использования добавьте в .env:"
echo "  TLS_CERT_PATH=$CERT_FILE"
echo "  TLS_KEY_PATH=$KEY_FILE"

