# Генерация самоподписанного сертификата для локальной разработки (PowerShell)

$CERT_DIR = "certs"
$CERT_FILE = "$CERT_DIR\localhost.crt"
$KEY_FILE = "$CERT_DIR\localhost.key"

# Создаем директорию для сертификатов
New-Item -ItemType Directory -Force -Path $CERT_DIR | Out-Null

# Генерируем самоподписанный сертификат
openssl req -x509 -newkey rsa:4096 -nodes `
    -keyout $KEY_FILE `
    -out $CERT_FILE `
    -days 365 `
    -subj "/C=RU/ST=State/L=City/O=Organization/CN=localhost" `
    -addext "subjectAltName=DNS:localhost,DNS:*.localhost,IP:127.0.0.1,IP:::1"

Write-Host "Сертификат создан:"
Write-Host "  Certificate: $CERT_FILE"
Write-Host "  Private Key: $KEY_FILE"
Write-Host ""
Write-Host "Для использования добавьте в .env:"
Write-Host "  TLS_CERT_PATH=$CERT_FILE"
Write-Host "  TLS_KEY_PATH=$KEY_FILE"

